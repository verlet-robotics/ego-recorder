#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")"

echo "=== Configure ==="
cmake -B build -DWITH_GUI=ON -DWITH_PYTHON=ON --log-level=STATUS \
    -DFETCHCONTENT_QUIET=OFF

echo ""
echo "=== Build ($(nproc) jobs) ==="
cmake --build build -j"$(nproc)" -- VERBOSE=0

echo ""
echo "=== Test ==="
cd build && ctest --output-on-failure --progress
