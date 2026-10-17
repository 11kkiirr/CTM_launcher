#!/bin/sh
set -eu

bundle_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
bin_dir="$HOME/.local/bin"
apps_dir="$data_home/applications"
icons_dir="$data_home/icons/hicolor/scalable/apps"

install -Dm755 "$bundle_dir/ctmlauncher" "$bin_dir/ctmlauncher"
install -Dm644 "$bundle_dir/ctmlauncher.desktop" "$apps_dir/ctmlauncher.desktop"
install -Dm644 "$bundle_dir/ctmlauncher.svg" "$icons_dir/ctmlauncher.svg"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$apps_dir" >/dev/null 2>&1 || true
fi

printf '%s\n' "CTMLauncher installed. Open it from the application menu or run ctmlauncher."
