#!/usr/bin/env bash

set -uo pipefail

usage() {
    echo "Usage: $0 [CONFIG_PATH]"
    echo "Starts EdgeMouse and records both a latest log and a timestamped archive."
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    usage
    exit 0
elif [[ $# -gt 1 ]]; then
    usage >&2
    exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "${script_dir}/.." && pwd)"
binary_path="${project_root}/target/release/edgemouse"
config_path="${1:-${project_root}/edgemouse.toml}"
log_dir="${project_root}/logs"
current_log="${project_root}/mac-current.log"
timestamp="$(date '+%Y%m%d-%H%M%S')"
archive_log="${log_dir}/mac-${timestamp}.log"

if [[ ! -x "${binary_path}" ]]; then
    echo "EdgeMouse executable not found: ${binary_path}" >&2
    echo "Build it first with: cargo build --release -p edgemouse-agent" >&2
    exit 1
fi

if [[ ! -f "${config_path}" ]]; then
    echo "EdgeMouse configuration not found: ${config_path}" >&2
    exit 1
fi

mkdir -p "${log_dir}"
cd "${project_root}"

set +e
{
    echo "EdgeMouse macOS session"
    echo "Started : $(date '+%Y-%m-%d %H:%M:%S %Z')"
    echo "Config  : ${config_path}"
    echo "Current : ${current_log}"
    echo "Archive : ${archive_log}"
    "${binary_path}" version
    echo
    if ! "${binary_path}" check-config "${config_path}"; then
        echo "Configuration check failed; EdgeMouse was not started."
        exit 1
    fi
    echo
    "${binary_path}" run "${config_path}"
} 2>&1 | tee "${current_log}" "${archive_log}"
exit_code=${PIPESTATUS[0]}
set -e

echo
echo "Latest log : ${current_log}"
echo "Archive log: ${archive_log}"
exit "${exit_code}"
