#!/usr/bin/env bash
set -euo pipefail

# Text colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

project_name="$(basename "$PWD")"
bin_name="$project_name"

log() {
  local color="$1"
  local msg="$2"
  echo -e "${color}${msg}${NC}"
}

show_help() {
  echo -e "${CYAN}Usage:${NC} ./build.sh [options]"
  echo ""
  echo -e "${YELLOW}Options:${NC}"
  echo "  --linux                  Build the Linux application binary"
  echo "  --windows                Build the Windows application binary"
  echo "  --setup                  Setup both the builder container images"
  echo "  --setup-linux-image      Setup the Linux builder container image"
  echo "  --setup-windows-image    Setup the Windows builder container image"
  echo "  --update-linux-image     Update packages and Rust in the Linux container image"
  echo "  --update-windows-image   Update packages and Rust in the Windows container image"
  echo "  --help                   Show this help message"
}

setup_volume() {
  if podman volume inspect cargo-cache >/dev/null 2>&1; then
    log "$BLUE" "--> Volume 'cargo-cache' already exists. Skipping."
  else
    log "$YELLOW" "--> Creating cargo-cache volume..."
    podman volume create cargo-cache
  fi
}

build_container_image() {
  local tag=$1
  local file=$2

  # Ensure volume exists before building/using images
  setup_volume

  log "$CYAN" "--> Building container image: $tag from $file"
  if podman build --pull=newer --layers=false --tag="$tag" --file="$file" .; then
    log "$GREEN" "OK: $tag built successfully."
  else
    log "$RED" "ERROR: Failed to build $tag."
    exit 1
  fi
}

update_container_image() {
  local tag=$1
  log "$CYAN" "--> Updating packages and Rust in container image: $tag"
  if podman run -it --name image_patch "$tag" bash -c "dnf -y update && rustup update"; then
    podman commit image_patch "$tag"
    podman rm image_patch
    log "$GREEN" "OK: $tag updated and committed."
  else
    podman rm image_patch || true
    log "$RED" "ERROR: Failed to update $tag."
    exit 1
  fi
}

build_linux_application_binary() {
  log "$YELLOW" "--> Starting Linux Application Build..."
  podman run --rm \
    -v cargo-cache:/cache/cargo \
    -v "$PWD":/work:Z \
    -w /work \
    rust_linux_builder \
    bash -lc "
      set -euo pipefail
      export CARGO_HOME=/cache/cargo/home
      export CARGO_TARGET_DIR=/cache/cargo/target/$project_name
      mkdir -p \"\$CARGO_HOME\" \"\$CARGO_TARGET_DIR\" dist
      cargo fmt
      cargo clippy
      cargo build --release
      rsync -a \"\$CARGO_TARGET_DIR/release/$bin_name\" \"dist/$bin_name\"
    "
  log "$GREEN" "SUCCESS: Linux binary is in dist/$bin_name"
}

build_windows_application_binary() {
  log "$YELLOW" "--> Starting Windows Application Build..."
  podman run --rm \
    -v cargo-cache:/cache/cargo \
    -v "$PWD":/work:Z \
    -w /work \
    rust_windows_builder \
    bash -lc "
      set -euo pipefail
      export CARGO_HOME=/cache/cargo/home
      export CARGO_TARGET_DIR=/cache/cargo/target/$project_name
      mkdir -p \"\$CARGO_HOME\" \"\$CARGO_TARGET_DIR\" dist
      export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
      cargo fmt
      cargo clippy
      cargo build --release --target x86_64-pc-windows-gnu
      rsync -a \"\$CARGO_TARGET_DIR/x86_64-pc-windows-gnu/release/$bin_name.exe\" \"dist/$bin_name.exe\"
    "
  log "$GREEN" "SUCCESS: Windows binary is in dist/$bin_name.exe"
}

if [[ $# -eq 0 ]]; then
  show_help
  exit 1
fi

while [[ $# -gt 0 ]]; do
  case "$1" in
  --setup)
    build_container_image rust_linux_builder Containerfile.linux_builder
    build_container_image rust_windows_builder Containerfile.windows_builder
    shift
    ;;
  --setup-linux-image)
    build_container_image rust_linux_builder Containerfile.linux_builder
    shift
    ;;
  --setup-windows-image)
    build_container_image rust_windows_builder Containerfile.windows_builder
    shift
    ;;
  --update-linux-image)
    update_container_image rust_linux_builder
    shift
    ;;
  --update-windows-image)
    update_container_image rust_windows_builder
    shift
    ;;
  --linux)
    mkdir -p dist
    build_linux_application_binary
    shift
    ;;
  --windows)
    mkdir -p dist
    build_windows_application_binary
    shift
    ;;
  --help)
    show_help
    exit 0
    ;;
  *)
    log "$RED" "Unknown option: $1"
    show_help
    exit 1
    ;;
  esac
done
