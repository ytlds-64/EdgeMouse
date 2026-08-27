#!/usr/bin/env bash

set -euo pipefail

usage() {
    echo "Usage: $0 {install|start|stop|status|uninstall} [CONFIG_PATH]"
    echo "Manages EdgeMouse as a macOS login agent."
}

if [[ $# -lt 1 || $# -gt 2 ]]; then
    usage >&2
    exit 2
fi

action="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "${script_dir}/.." && pwd)"
binary_path="${project_root}/target/release/edgemouse"
config_path="${2:-${project_root}/edgemouse.toml}"
if [[ "${config_path}" != /* ]]; then
    config_path="${project_root}/${config_path}"
fi

label="com.edgemouse.agent"
domain="gui/$(id -u)"
service="${domain}/${label}"
plist_path="${HOME}/Library/LaunchAgents/${label}.plist"
log_dir="${project_root}/logs"
stdout_log="${log_dir}/mac-autostart.out.log"
stderr_log="${log_dir}/mac-autostart.err.log"

xml_escape() {
    printf '%s' "$1" | sed \
        -e 's/&/\&amp;/g' \
        -e 's/</\&lt;/g' \
        -e 's/>/\&gt;/g' \
        -e 's/"/\&quot;/g' \
        -e "s/'/\\&apos;/g"
}

is_loaded() {
    launchctl print "${service}" >/dev/null 2>&1
}

validate_installation() {
    if [[ ! -x "${binary_path}" ]]; then
        echo "EdgeMouse executable not found: ${binary_path}" >&2
        echo "Build it first with: cargo build --release -p edgemouse-agent" >&2
        exit 1
    fi
    if [[ ! -f "${config_path}" ]]; then
        echo "EdgeMouse configuration not found: ${config_path}" >&2
        exit 1
    fi
    "${binary_path}" version
    "${binary_path}" check-config "${config_path}"
}

write_plist() {
    mkdir -p "$(dirname "${plist_path}")" "${log_dir}"
    local binary_xml config_xml root_xml stdout_xml stderr_xml
    binary_xml="$(xml_escape "${binary_path}")"
    config_xml="$(xml_escape "${config_path}")"
    root_xml="$(xml_escape "${project_root}")"
    stdout_xml="$(xml_escape "${stdout_log}")"
    stderr_xml="$(xml_escape "${stderr_log}")"
    local temporary_plist="${plist_path}.tmp.$$"
    trap 'rm -f "${temporary_plist}"' RETURN
    {
        echo '<?xml version="1.0" encoding="UTF-8"?>'
        echo '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">'
        echo '<plist version="1.0">'
        echo '<dict>'
        echo '    <key>Label</key>'
        echo "    <string>${label}</string>"
        echo '    <key>ProgramArguments</key>'
        echo '    <array>'
        echo "        <string>${binary_xml}</string>"
        echo '        <string>run</string>'
        echo "        <string>${config_xml}</string>"
        echo '    </array>'
        echo '    <key>WorkingDirectory</key>'
        echo "    <string>${root_xml}</string>"
        echo '    <key>RunAtLoad</key>'
        echo '    <true/>'
        echo '    <key>KeepAlive</key>'
        echo '    <dict>'
        echo '        <key>SuccessfulExit</key>'
        echo '        <false/>'
        echo '    </dict>'
        echo '    <key>ThrottleInterval</key>'
        echo '    <integer>5</integer>'
        echo '    <key>ProcessType</key>'
        echo '    <string>Interactive</string>'
        echo '    <key>LimitLoadToSessionType</key>'
        echo '    <string>Aqua</string>'
        echo '    <key>StandardOutPath</key>'
        echo "    <string>${stdout_xml}</string>"
        echo '    <key>StandardErrorPath</key>'
        echo "    <string>${stderr_xml}</string>"
        echo '</dict>'
        echo '</plist>'
    } >"${temporary_plist}"
    plutil -lint "${temporary_plist}" >/dev/null
    mv "${temporary_plist}" "${plist_path}"
    trap - RETURN
}

safe_stop() {
    if [[ -x "${binary_path}" ]]; then
        "${binary_path}" stop
    fi
}

show_status() {
    if is_loaded; then
        echo "Login startup: installed and loaded"
    elif [[ -f "${plist_path}" ]]; then
        echo "Login startup: installed but not loaded"
    else
        echo "Login startup: not installed"
    fi
    if [[ -x "${binary_path}" ]]; then
        "${binary_path}" status
    else
        echo "EdgeMouse executable not found: ${binary_path}"
    fi
    echo "Output log : ${stdout_log}"
    echo "Error log  : ${stderr_log}"
}

case "${action}" in
    install)
        validate_installation
        safe_stop
        if is_loaded; then
            launchctl bootout "${service}" >/dev/null 2>&1 || true
        fi
        write_plist
        launchctl bootstrap "${domain}" "${plist_path}"
        launchctl kickstart "${service}"
        sleep 1
        echo "EdgeMouse login startup installed"
        show_status
        ;;
    start)
        validate_installation
        if "${binary_path}" status | grep -q 'is running'; then
            echo "EdgeMouse is already running"
        else
            if ! is_loaded; then
                if [[ ! -f "${plist_path}" ]]; then
                    echo "Login startup is not installed; run: $0 install \"${config_path}\"" >&2
                    exit 1
                fi
                launchctl bootstrap "${domain}" "${plist_path}"
            fi
            launchctl kickstart "${service}"
            sleep 1
            echo "EdgeMouse start requested"
        fi
        show_status
        ;;
    stop)
        safe_stop
        show_status
        ;;
    status)
        show_status
        ;;
    uninstall)
        safe_stop
        if is_loaded; then
            launchctl bootout "${service}" >/dev/null
        fi
        rm -f "${plist_path}"
        echo "EdgeMouse login startup removed"
        show_status
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
