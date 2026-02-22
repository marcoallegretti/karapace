#!/usr/bin/env bash
set -euo pipefail

command -v cargo-cyclonedx >/dev/null 2>&1 || {
    echo "Installing cargo-cyclonedx..."
    cargo install cargo-cyclonedx@0.5.7 --locked
}

cargo cyclonedx --format json --override-filename karapace_bom
echo "SBOM written to karapace_bom.cdx.json"
