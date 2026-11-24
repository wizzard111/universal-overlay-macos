#!/bin/bash
# Universal Overlay — update (macOS). The Terminal window is the log.
#
# In a git clone of the repository: pulls the latest changes, rebuilds and
# installs in update mode. Otherwise runs the updater: it downloads the latest
# GitHub release, verifies its SHA-256 and replaces the app (settings are kept).
set -u
printf '\033]0;Universal Overlay — update\007'
ROOT=$(cd "$(dirname "$0")" && pwd)
APP="/Applications/Universal Overlay.app"

if [ -d "$ROOT/.git" ] && [ -f "$ROOT/source/Cargo.toml" ] && command -v git >/dev/null && command -v cargo >/dev/null; then
    echo "Pulling the latest changes…"
    git -C "$ROOT" pull --ff-only || exit 1
    PKG=$(sh "$ROOT/source/build/build.sh" --print-package | tail -n 1) || exit 1
    exec /bin/bash "$PKG/install.command" --update
fi

# Inside the .app (Contents/Resources) or next to it.
for u in "$ROOT/../MacOS/universal-overlay-updater" "$ROOT/app/Universal Overlay.app/Contents/MacOS/universal-overlay-updater" \
         "$APP/Contents/MacOS/universal-overlay-updater"; do
    if [ -x "$u" ]; then
        "$u" --install-dir "$APP"
        exit $?
    fi
done
echo "Universal Overlay is not installed. Open install.command."
exit 1
