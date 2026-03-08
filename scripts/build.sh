#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."

echo "=== Configure ==="
cmake -B build -DWITH_GUI=ON -DWITH_PYTHON=ON --log-level=STATUS \
    -DFETCHCONTENT_QUIET=OFF

echo ""
echo "=== Build ($(nproc) jobs) ==="
cmake --build build -j"$(nproc)" -- VERBOSE=0

echo ""
echo "=== Test ==="
cd build && ctest --output-on-failure --progress
cd ..

# Install if the installed binary is outdated or missing
INSTALLED="/usr/local/bin/ego-recorder"
BUILT="build/ego-recorder"
if [ -f "$BUILT" ]; then
    if [ ! -f "$INSTALLED" ] || [ "$BUILT" -nt "$INSTALLED" ]; then
        echo ""
        echo "=== Install ==="
        sudo cmake --install build
    fi
fi
