#!/bin/sh
set -eu

project_root="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
target_name="${1:-$(rustc -vV | awk '/^host:/ { print $2 }')}"
version="$(awk -F '"' '/^version = "/ { print $2; exit }' "${project_root}/Cargo.toml")"

case "${target_name}" in
  aarch64-apple-darwin) architecture="aarch64" ;;
  x86_64-apple-darwin) architecture="x64" ;;
  universal-apple-darwin) architecture="universal" ;;
  *)
    echo "Unsupported macOS target: ${target_name}" >&2
    exit 1
    ;;
esac

bundle_root="${project_root}/target/${target_name}/release/bundle"
application="${bundle_root}/macos/EdgeMouse.app"
output_directory="${bundle_root}/dmg"
output="${output_directory}/EdgeMouse_${version}_${architecture}.dmg"

if [ ! -d "${application}" ]; then
  echo "EdgeMouse.app was not found at ${application}" >&2
  echo "Build the app bundle before creating the DMG." >&2
  exit 1
fi

staging_directory="$(mktemp -d "${TMPDIR:-/tmp}/edgemouse-dmg.XXXXXX")"
trap 'rm -rf "${staging_directory}"' EXIT HUP INT TERM

mkdir -p "${output_directory}"
ditto "${application}" "${staging_directory}/EdgeMouse.app"
ln -s /Applications "${staging_directory}/Applications"
hdiutil create \
  -volname EdgeMouse \
  -srcfolder "${staging_directory}" \
  -ov \
  -format UDZO \
  "${output}"

echo "Created EdgeMouse installer: ${output}"
