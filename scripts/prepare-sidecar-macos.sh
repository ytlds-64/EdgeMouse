#!/bin/sh
set -eu

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
binary_directory="${project_root}/crates/edgemouse-desktop/binaries"
target_name="${1:-$(rustc -vV | awk '/^host:/ { print $2 }')}"

mkdir -p "${binary_directory}"

if [ "${target_name}" = "universal-apple-darwin" ]; then
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo build --manifest-path "${project_root}/Cargo.toml" --release -p edgemouse-agent --target aarch64-apple-darwin
  cargo build --manifest-path "${project_root}/Cargo.toml" --release -p edgemouse-agent --target x86_64-apple-darwin

  # Tauri's universal build compiles the app once per architecture before
  # merging both app bundles. Each pass resolves externalBin using its own
  # target triple, so keep the thin sidecars alongside the universal binary.
  cp \
    "${project_root}/target/aarch64-apple-darwin/release/edgemouse" \
    "${binary_directory}/edgemouse-aarch64-apple-darwin"
  cp \
    "${project_root}/target/x86_64-apple-darwin/release/edgemouse" \
    "${binary_directory}/edgemouse-x86_64-apple-darwin"
  lipo -create \
    "${project_root}/target/aarch64-apple-darwin/release/edgemouse" \
    "${project_root}/target/x86_64-apple-darwin/release/edgemouse" \
    -output "${binary_directory}/edgemouse-universal-apple-darwin"

  chmod 755 \
    "${binary_directory}/edgemouse-aarch64-apple-darwin" \
    "${binary_directory}/edgemouse-x86_64-apple-darwin"
else
  cargo build --manifest-path "${project_root}/Cargo.toml" --release -p edgemouse-agent --target "${target_name}"
  cp "${project_root}/target/${target_name}/release/edgemouse" "${binary_directory}/edgemouse-${target_name}"
fi

chmod 755 "${binary_directory}/edgemouse-${target_name}"
echo "Prepared EdgeMouse sidecar: ${binary_directory}/edgemouse-${target_name}"
