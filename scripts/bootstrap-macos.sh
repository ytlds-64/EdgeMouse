#!/usr/bin/env bash

set -euo pipefail

verify_only=false
if [[ "${1:-}" == "--verify-only" ]]; then
    verify_only=true
elif [[ $# -ne 0 ]]; then
    echo "Usage: $0 [--verify-only]" >&2
    exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "${script_dir}/.." && pwd)"
cd "${project_root}"

for required_command in cargo rustc rustup; do
    if ! command -v "${required_command}" >/dev/null 2>&1; then
        echo "Missing ${required_command}. Install Rust from https://rustup.rs and rerun this script." >&2
        exit 1
    fi
done

echo "==> Rust toolchain"
rustc --version
cargo --version
rustup component add rustfmt clippy

echo "==> Formatting"
cargo fmt --all -- --check

echo "==> Static analysis"
cargo clippy --workspace --all-targets -- -D warnings

echo "==> Tests"
cargo test --workspace

echo "==> Release build"
cargo build --release -p edgemouse-agent

binary_path="${project_root}/target/release/edgemouse"
if [[ ! -x "${binary_path}" ]]; then
    echo "Release executable was not created at ${binary_path}." >&2
    exit 1
fi

echo "==> Platform diagnostics"
"${binary_path}" doctor

if [[ "${verify_only}" == true ]]; then
    echo
    echo "Verification completed. Identity and configuration were not changed."
    exit 0
fi

identity_dir="${project_root}/mac-identity"
certificate_path="${identity_dir}/certificate.der"
private_key_path="${identity_dir}/private-key.der"

if [[ -f "${certificate_path}" && -f "${private_key_path}" ]]; then
    echo "==> Existing macOS identity kept"
elif [[ -e "${certificate_path}" || -e "${private_key_path}" ]]; then
    echo "The mac-identity directory is incomplete. Move it aside, then rerun this script." >&2
    exit 1
else
    echo "==> Generating macOS identity"
    "${binary_path}" identity "${identity_dir}"
fi

config_path="${project_root}/edgemouse.toml"
if [[ -e "${config_path}" ]]; then
    echo "==> Existing edgemouse.toml kept"
else
    echo "==> Creating edgemouse.toml from the macOS template"
    cp "${project_root}/examples/macos.toml" "${config_path}"
fi

echo
echo "macOS preparation completed."
echo "Executable : ${binary_path}"
echo "Certificate: ${certificate_path}"
echo "Config     : ${config_path}"
echo
echo "Manual steps still required:"
echo "1. Edit edgemouse.toml: both screen sizes and layout.peer_on. Keep peer.address as auto, or set a static LAN address."
echo "2. On Windows, allow TCP 43893 and UDP 43892, then run its 'pair host' command."
echo "3. Stop any running EdgeMouse agent, then enter the Windows code with: ${binary_path} pair join ${config_path} CODE"
echo "4. Grant EdgeMouse Accessibility access in macOS System Settings."
echo "5. Run: ${binary_path} check-config ${config_path}"
echo "6. Run: ${binary_path} run ${config_path}"
