#!/usr/bin/env bash

set -euo pipefail

usage() {
    echo "Usage: $0 {install|status}"
    echo "Creates the fixed local signing identity used by EdgeMouse on this Mac."
}

if [[ $# -ne 1 ]]; then
    usage >&2
    exit 2
fi

action="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
identity_name="EdgeMouse Local Code Signing"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
openssl_config="${script_dir}/macos-local-signing-openssl.cnf"
login_keychain="${HOME}/Library/Keychains/login.keychain-db"

find_identity() {
    /usr/bin/security find-identity -v -p codesigning 2>/dev/null | \
        /usr/bin/grep -F "\"${identity_name}\"" | \
        /usr/bin/head -n 1 || true
}

show_status() {
    local identity
    identity="$(find_identity)"
    if [[ -n "${identity}" ]]; then
        echo "EdgeMouse fixed signing identity is ready"
        echo "${identity}"
    else
        echo "EdgeMouse fixed signing identity is not installed"
        return 1
    fi
}

install_identity() {
    if show_status >/dev/null 2>&1; then
        show_status
        return
    fi
    if [[ ! -f "${openssl_config}" ]]; then
        echo "OpenSSL configuration not found: ${openssl_config}" >&2
        exit 1
    fi
    if [[ ! -f "${login_keychain}" ]]; then
        echo "Login keychain not found: ${login_keychain}" >&2
        exit 1
    fi

    local temp_dir private_key certificate identity_archive archive_password
    temp_dir="$(mktemp -d "${TMPDIR:-/tmp}/edgemouse-signing.XXXXXX")"
    private_key="${temp_dir}/private-key.pem"
    certificate="${temp_dir}/certificate.pem"
    identity_archive="${temp_dir}/identity.p12"
    archive_password="$(/usr/bin/openssl rand -hex 24)"
    cleanup() {
        rm -f "${private_key}" "${certificate}" "${identity_archive}"
        rmdir "${temp_dir}" 2>/dev/null || true
    }
    trap cleanup RETURN

    if /usr/bin/security find-certificate \
        -c "${identity_name}" \
        "${login_keychain}" >/dev/null 2>&1; then
        /usr/bin/security find-certificate \
            -c "${identity_name}" \
            -p \
            "${login_keychain}" >"${certificate}"
        /usr/bin/security add-trusted-cert \
            -r trustRoot \
            -p codeSign \
            -k "${login_keychain}" \
            "${certificate}"
        echo "Existing EdgeMouse certificate trusted for code signing"
        show_status
        return
    fi

    /usr/bin/openssl req \
        -new \
        -newkey rsa:3072 \
        -x509 \
        -sha256 \
        -days 3650 \
        -nodes \
        -config "${openssl_config}" \
        -keyout "${private_key}" \
        -out "${certificate}" >/dev/null 2>&1
    /usr/bin/openssl pkcs12 \
        -export \
        -name "${identity_name}" \
        -inkey "${private_key}" \
        -in "${certificate}" \
        -out "${identity_archive}" \
        -passout "pass:${archive_password}" >/dev/null 2>&1

    /usr/bin/security import "${identity_archive}" \
        -k "${login_keychain}" \
        -P "${archive_password}" \
        -T /usr/bin/codesign >/dev/null
    /usr/bin/security add-trusted-cert \
        -r trustRoot \
        -p codeSign \
        -k "${login_keychain}" \
        "${certificate}"

    echo "EdgeMouse fixed local signing identity installed"
    show_status
    echo "Future EdgeMouse builds on this Mac will keep the same Privacy & Security identity."
}

case "${action}" in
    install)
        install_identity
        ;;
    status)
        show_status
        ;;
    *)
        usage >&2
        exit 2
        ;;
esac
