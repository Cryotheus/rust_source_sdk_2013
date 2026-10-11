#!/usr/bin/env bash
set -euo pipefail
fixture_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
output_dir=$(mktemp -d)
trap 'rm -rf -- "$output_dir"' EXIT
for channel in stable dev; do
    if [[ "$channel" == stable ]]; then headers="${METAMOD_SOURCE_STABLE:?}"; define=METAMOD_BRIDGE_STABLE; else headers="${METAMOD_SOURCE_DEV:?}"; define=METAMOD_BRIDGE_DEV; fi
    "${CXX:-c++}" -std=c++17 -DMETA_NO_HL2SDK -D"$define" -I"$headers/core" -I"$headers/core/sourcehook" -I"$headers/third_party/khook/include" "$fixture_dir/level_generation.cpp" -o "$output_dir/$channel"
    "$output_dir/$channel"
done
