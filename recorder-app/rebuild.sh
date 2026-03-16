#!/usr/bin/env bash
# rebuild.sh -- Rebuild and install the recorder app after code changes.
#
# Rebuilds whichever components have changed:
#   1. C++ ego-recorder binary (if src/ changed)
#   2. Rust/Tauri backend + React frontend (recorder-app)
#
# Usage:
#   ./rebuild.sh            # Rebuild everything that changed
#   ./rebuild.sh --cpp      # Only C++ recorder
#   ./rebuild.sh --app      # Only Tauri app
#   ./rebuild.sh --release  # Build Tauri in release mode
#   ./rebuild.sh --test     # Run tests after building

set -euo pipefail

BOLD='\033[1m'
GREEN='\033[0;32m'
CYAN='\033[0;36m'
YELLOW='\033[0;33m'
RED='\033[0;31m'
DIM='\033[2m'
NC='\033[0m'

info()  { echo -e "${CYAN}[rebuild]${NC} $*"; }
ok()    { echo -e "${GREEN}[rebuild]${NC} $*"; }
warn()  { echo -e "${YELLOW}[rebuild]${NC} $*"; }
err()   { echo -e "${RED}[rebuild]${NC} $*" >&2; }

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_DIR="${PROJECT_DIR}/build"
APP_DIR="${PROJECT_DIR}/recorder-app"
NPROC="$(nproc 2>/dev/null || echo 4)"

BUILD_CPP=true
BUILD_APP=true
RELEASE=false
RUN_TESTS=false
CLEAN=false

while [[ $# -gt 0 ]]; do
    case "$1" in
        --cpp)     BUILD_CPP=true; BUILD_APP=false; shift ;;
        --app)     BUILD_CPP=false; BUILD_APP=true; shift ;;
        --release) RELEASE=true; shift ;;
        --test)    RUN_TESTS=true; shift ;;
        --clean)   CLEAN=true; shift ;;
        --help|-h)
            echo "Usage: ./rebuild.sh [OPTIONS]"
            echo ""
            echo "Options:"
            echo "  --cpp       Only rebuild C++ ego-recorder binary"
            echo "  --app       Only rebuild Tauri recorder app"
            echo "  --release   Build Tauri app in release mode"
            echo "  --test      Run tests after building"
            echo "  --clean     Delete build directories before rebuilding"
            echo ""
            echo "With no flags, rebuilds both C++ and Tauri app (debug mode)."
            exit 0 ;;
        *) err "Unknown option: $1"; exit 1 ;;
    esac
done

SECONDS=0

# ---------------------------------------------------------------------------
# 1. C++ ego-recorder
# ---------------------------------------------------------------------------
if [[ "$BUILD_CPP" == true ]]; then
    if [[ "$CLEAN" == true ]] && [[ -d "$BUILD_DIR" ]]; then
        info "Cleaning C++ build directory..."
        rm -rf "$BUILD_DIR"
    fi

    info "Building C++ ego-recorder..."

    if [[ ! -d "$BUILD_DIR" ]]; then
        info "Configuring CMake (first build)..."
        cmake -B "$BUILD_DIR" \
            -DWITH_GUI=ON \
            -DWITH_PYTHON=ON \
            -DFETCHCONTENT_BASE_DIR="${PROJECT_DIR}/.deps" \
            "$PROJECT_DIR"
    fi

    cmake --build "$BUILD_DIR" --parallel "$NPROC"

    # Install to /usr/local/bin if the installed binary is outdated
    INSTALLED="/usr/local/bin/ego-recorder"
    BUILT="${BUILD_DIR}/ego-recorder"
    if [[ -f "$BUILT" ]]; then
        if [[ ! -f "$INSTALLED" ]] || [[ "$BUILT" -nt "$INSTALLED" ]]; then
            info "Installing ego-recorder to ${INSTALLED}..."
            sudo cp "$BUILT" "$INSTALLED"
            ok "Installed ego-recorder"
        else
            ok "ego-recorder binary up to date"
        fi
    fi

    if [[ "$RUN_TESTS" == true ]]; then
        info "Running C++ tests..."
        (cd "$BUILD_DIR" && ctest --output-on-failure --progress)
        ok "C++ tests passed"
    fi
fi

# ---------------------------------------------------------------------------
# 2. Tauri recorder app (Rust backend + React frontend)
# ---------------------------------------------------------------------------
if [[ "$BUILD_APP" == true ]]; then
    if [[ "$CLEAN" == true ]]; then
        info "Cleaning Tauri build artifacts..."
        rm -rf "${APP_DIR}/src-tauri/target"
    fi

    info "Building recorder app..."

    # Ensure Bun is in PATH
    if [[ -d "${HOME}/.bun/bin" ]]; then
        export PATH="${HOME}/.bun/bin:${PATH}"
    fi
    if ! command -v bun &>/dev/null; then
        err "Bun not found. Run setup-station.sh first."
        exit 1
    fi

    # Ensure Rust/cargo is in PATH
    if ! command -v cargo &>/dev/null && [[ -f "${HOME}/.cargo/env" ]]; then
        source "${HOME}/.cargo/env"
    fi
    if ! command -v cargo &>/dev/null; then
        err "cargo not found. Run setup-station.sh first."
        exit 1
    fi

    # Install frontend deps if needed
    if [[ ! -d "${APP_DIR}/node_modules" ]]; then
        info "Installing frontend dependencies..."
        (cd "$APP_DIR" && bun install)
    fi

    if [[ "$RELEASE" == true ]]; then
        info "Building Tauri app (release)..."
        (cd "$APP_DIR" && bun run tauri build 2>&1)

        # Find the built binary
        local_bin="${HOME}/.local/bin"
        built_bin="${APP_DIR}/src-tauri/target/release/ego-recorder-app"
        if [[ -f "$built_bin" ]]; then
            mkdir -p "$local_bin"
            cp "$built_bin" "${local_bin}/ego-recorder-app"
            ok "Installed ego-recorder-app to ${local_bin}/ego-recorder-app"

            # Check if ~/.local/bin is in PATH
            if ! echo "$PATH" | grep -q "${local_bin}"; then
                warn "Add to your shell profile: export PATH=\"\$HOME/.local/bin:\$PATH\""
            fi
        fi
    else
        info "Building Tauri app (debug)..."
        (cd "${APP_DIR}/src-tauri" && cargo build)
        ok "Debug build: ${APP_DIR}/src-tauri/target/debug/ego-recorder-app"
    fi

    if [[ "$RUN_TESTS" == true ]]; then
        info "Running Rust tests..."
        (cd "${APP_DIR}/src-tauri" && cargo test)
        ok "Rust tests passed"

        info "Running TypeScript type check..."
        (cd "$APP_DIR" && npx tsc --noEmit)
        ok "TypeScript check passed"
    fi
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
echo -e "${GREEN}${BOLD}  Rebuild complete${NC} ${DIM}(${SECONDS}s)${NC}"
[[ "$BUILD_CPP" == true ]] && echo -e "  C++ binary: ${BUILD_DIR}/ego-recorder"
if [[ "$BUILD_APP" == true ]]; then
    if [[ "$RELEASE" == true ]]; then
        echo -e "  Tauri app:  ${HOME}/.local/bin/ego-recorder-app"
    else
        echo -e "  Tauri app:  ${APP_DIR}/src-tauri/target/debug/ego-recorder-app"
        echo -e ""
        echo -e "  ${DIM}For dev mode with hot-reload:  cd recorder-app && bun run tauri dev${NC}"
        echo -e "  ${DIM}For release build:             ./rebuild.sh --release${NC}"
    fi
fi
echo ""
