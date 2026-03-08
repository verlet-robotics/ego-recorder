#!/usr/bin/env bash
# Wrapper -- delegates to scripts/setup.sh
exec "$(dirname "$0")/scripts/setup.sh" "$@"
