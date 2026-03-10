#!/usr/bin/env bash
# lib-env.sh -- Shared helpers for R2 credential setup and facility detection.
#
# Sourced by setup scripts. Requires color functions (info, ok, warn, err)
# and BOLD/NC variables to be defined by the calling script.

# ---------------------------------------------------------------------------
# Auto-detect facility server on local /24 subnet
# ---------------------------------------------------------------------------
# Pure function: echoes the IP of a facility server if found.
# Returns 0 if found, 1 if not. No side-effect output (safe for $()).
detect_facility_server() {
    local local_ip
    local_ip=$(ip route get 1.1.1.1 2>/dev/null \
        | awk '{for(i=1;i<=NF;i++) if($i=="src") print $(i+1); exit}')
    [[ -z "$local_ip" ]] && return 1

    local subnet="${local_ip%.*}"
    local result_file
    result_file=$(mktemp)

    # Parallel TCP connect to port 8100 across the /24
    local i
    for i in $(seq 1 254); do
        [[ "${subnet}.$i" == "$local_ip" ]] && continue
        (
            if timeout 1 bash -c "echo >/dev/tcp/${subnet}.$i/8100" 2>/dev/null; then
                echo "${subnet}.$i" >> "$result_file"
            fi
        ) &
    done
    wait

    # Verify candidates with an HTTP probe
    local found=""
    if [[ -s "$result_file" ]]; then
        while IFS= read -r candidate; do
            if curl -sf "http://${candidate}:8100/health" --max-time 2 >/dev/null 2>&1 ||
               curl -sf "http://${candidate}:8100/" --max-time 2 >/dev/null 2>&1; then
                found="$candidate"
                break
            fi
        done < "$result_file"
    fi
    rm -f "$result_file"

    [[ -n "$found" ]] && echo "$found" && return 0
    return 1
}

# ---------------------------------------------------------------------------
# R2 credential setup (paste-block or individual entry)
# ---------------------------------------------------------------------------
# Usage: prompt_r2_credentials /path/to/.env [sudo]
#   Pass "sudo" as second arg to write with elevated privileges.
#   Returns 0 if credentials written, 1 if skipped.
prompt_r2_credentials() {
    local env_file="$1"
    local use_sudo="${2:-}"

    if [[ -f "$env_file" ]]; then
        info ".env already exists at ${env_file}"
        read -rp "  Overwrite? [y/N] " ans
        if [[ ! "$ans" =~ ^[Yy] ]]; then
            info "Keeping existing .env"
            return 0
        fi
    fi

    echo ""
    echo -e "${BOLD}R2 Cloud Upload Credentials${NC}"
    echo "─────────────────────────────────────"
    echo ""
    echo "The uploader needs Cloudflare R2 credentials to sync recordings."
    echo "Credentials will be stored in ${env_file} (owner-readable only)."
    echo ""
    echo "  1) Paste a .env block (all credentials at once)"
    echo "  2) Enter each credential individually"
    echo "  3) Skip (configure later)"
    echo ""
    read -rp "Choice [1/2/3]: " choice

    local tmp_env
    tmp_env=$(mktemp)

    case "$choice" in
        1)
            echo ""
            echo "Paste your .env block below (blank line when done):"
            echo ""
            while IFS= read -r line; do
                [[ -z "$line" ]] && break
                # Strip 'export ' prefix, comments, whitespace
                line="${line#export }"
                line="${line%%#*}"
                line="$(echo "$line" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
                [[ -z "$line" || "$line" != *=* ]] && continue
                # Extract key and value, strip surrounding quotes
                local key="${line%%=*}"
                local val="${line#*=}"
                val="${val#\"}" ; val="${val%\"}"
                val="${val#\'}" ; val="${val%\'}"
                # Normalize alternate key names
                [[ "$key" == "R2_ENDPOINT_URL" ]] && key="R2_ENDPOINT"
                [[ "$key" == "R2_BUCKET_NAME" ]] && key="R2_BUCKET"
                echo "${key}=${val}" >> "$tmp_env"
            done

            if [[ ! -s "$tmp_env" ]]; then
                warn "No credentials pasted."
                rm -f "$tmp_env"
                return 1
            fi

            # Display parsed values (mask secrets)
            echo ""
            info "Parsed:"
            while IFS='=' read -r k v; do
                [[ -z "$k" ]] && continue
                if [[ "$k" == *SECRET* || "$k" == *PASSWORD* || "$k" == *KEY_ID* ]]; then
                    echo "    ${k} = ****${v: -4}"
                else
                    echo "    ${k} = ${v}"
                fi
            done < "$tmp_env"
            echo ""
            ;;
        2)
            read -rp "R2 Endpoint URL (e.g. https://<id>.r2.cloudflarestorage.com): " r2_ep
            read -rp "R2 Bucket Name: " r2_bucket
            read -rp "R2 Access Key ID: " r2_key
            read -rsp "R2 Secret Access Key: " r2_secret
            echo ""
            {
                echo "R2_ENDPOINT=${r2_ep}"
                echo "R2_BUCKET=${r2_bucket}"
                echo "R2_ACCESS_KEY_ID=${r2_key}"
                echo "R2_SECRET_ACCESS_KEY=${r2_secret}"
            } > "$tmp_env"

            if [[ -z "$r2_ep" || -z "$r2_bucket" || -z "$r2_key" || -z "$r2_secret" ]]; then
                warn "Some credentials are empty. Fill them in later: ${env_file}"
            fi
            ;;
        *)
            info "Skipping R2 credentials. Configure later: ${env_file}"
            rm -f "$tmp_env"
            return 1
            ;;
    esac

    # Write to target location
    local parent_dir
    parent_dir="$(dirname "$env_file")"
    if [[ "$use_sudo" == "sudo" ]]; then
        sudo mkdir -p "$parent_dir"
        sudo install -m 600 "$tmp_env" "$env_file"
        sudo chown root:root "$env_file"
    else
        mkdir -p "$parent_dir"
        install -m 600 "$tmp_env" "$env_file"
    fi
    rm -f "$tmp_env"

    ok "Credentials saved to ${env_file}"
    return 0
}

# ---------------------------------------------------------------------------
# Facility connection setup (auto-detect + prompt)
# ---------------------------------------------------------------------------
# Usage: setup_facility /path/to/.env [/path/to/upload_config.toml]
#   Auto-detects facility server on LAN, prompts if not found.
#   Adds FACILITY_URL to .env. Updates config.toml if provided.
#   Returns 0 if configured, 1 if skipped.
setup_facility() {
    local env_file="$1"
    local config_file="${2:-}"

    echo ""
    echo -e "${BOLD}Facility API Connection${NC}"
    echo "─────────────────────────────────────"
    echo ""
    echo "Connect to a facility server so recordings appear in the dashboard."
    echo ""

    local facility_ip=""

    # Auto-detection
    info "Scanning local network for facility server..."
    if facility_ip=$(detect_facility_server); then
        ok "Found facility server at ${facility_ip}"
        read -rp "Use ${facility_ip}? [Y/n] " ans
        [[ "$ans" =~ ^[Nn] ]] && facility_ip=""
    else
        info "No facility server found on local network."
    fi

    # Manual fallback
    if [[ -z "$facility_ip" ]]; then
        read -rp "Facility server IP or URL (leave empty to skip): " facility_ip
    fi

    if [[ -z "$facility_ip" ]]; then
        info "Skipping facility setup."
        return 1
    fi

    # Normalize URL
    local url="$facility_ip"
    [[ "$url" != http://* && "$url" != https://* ]] && url="http://${url}"
    [[ "$url" != *:[0-9]* ]] && url="${url}:8100"

    # Dataset name
    read -rp "Dataset name (default: $(hostname)): " ds_name
    ds_name="${ds_name:-$(hostname)}"

    # Add FACILITY_URL to .env if it exists
    if [[ -f "$env_file" ]]; then
        if grep -q "^FACILITY_URL=" "$env_file" 2>/dev/null; then
            sed -i "s|^FACILITY_URL=.*|FACILITY_URL=${url}|" "$env_file"
        else
            echo "FACILITY_URL=${url}" >> "$env_file"
        fi
    fi

    # Update upload_config.toml if provided
    if [[ -n "$config_file" && -f "$config_file" ]]; then
        sed -i "s|^enabled = .*|enabled = true|" "$config_file"
        sed -i "s|^url = .*|url = \"${url}\"|" "$config_file"
        sed -i "s|^dataset_name = .*|dataset_name = \"${ds_name}\"|" "$config_file"
    fi

    ok "Facility configured: ${url} (dataset: ${ds_name})"
    return 0
}
