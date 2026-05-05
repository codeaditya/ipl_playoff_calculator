#!/usr/bin/env bash
set -euo pipefail

## ----------------------------------------------------------------------------
## One time setup before running this build script
## ----------------------------------------------------------------------------
#
# Create the volume:
#     podman volume create cargo-cache
#
# Build a local image from the Containerfile:
#     podman build --pull=newer --layers=false --tag=manylinux_builder --file=Containerfile . && podman image prune -f
#
# They should show up here:
#     podman system df -v
#
## ----------------------------------------------------------------------------

# Release binaries would be available in the project's dist/ directory
# after the script finishes

project_name="$(basename "$PWD")"
bin_name="$project_name"

mkdir -p dist

podman run --rm \
  -v cargo-cache:/cache/cargo \
  -v "$PWD":/work:Z \
  -w /work \
  manylinux_builder \
  bash -lc '
    set -euo pipefail
    export CARGO_HOME=/cache/cargo/home
    export CARGO_TARGET_DIR=/cache/cargo/target/'"$project_name"'
    mkdir -p "$CARGO_HOME" "$CARGO_TARGET_DIR" dist
    cargo build --release
    rsync -a "$CARGO_TARGET_DIR/release/'"$bin_name"'" "dist/'"$bin_name"'"
  '
