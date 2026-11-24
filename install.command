#!/bin/bash
# Universal Overlay — install and update (macOS).
#
# Double-clicking opens Terminal — that window is the installation log.
#   From the DMG:  the disk window opens next to it; drag "Universal Overlay"
#                  onto "Applications" (or press Enter and it is copied for you).
#   From an archive: installs app/Universal Overlay.app.
#   From a clone of the repository: builds the program first (needs Rust and
#                  the Xcode Command Line Tools).
#
#   --update            update without questions (this is how the updater calls it)
#   --install-dir PATH  another location of the .app (default: /Applications)
#
# The administrator password is asked only when /Applications is not
# writable without it. macOS asks for the Screen Recording permission
# (needed for FPS) by itself.
set -u

APPNAME="Universal Overlay.app"
LABEL=com.universaloverlay.app
DEST="/Applications/$APPNAME"
MODE=install
prev=
for a in "$@"; do
    [ "$a" = --update ] && MODE=update
    [ "$prev" = --install-dir ] && DEST=$a
    prev=$a
done
ROOT=$(cd "$(dirname "$0")" && pwd)
LOGDIR="$HOME/Library/Logs/UniversalOverlay"
mkdir -p "$LOGDIR"
LOG="$LOGDIR/install.log"
AGENT="$HOME/Library/LaunchAgents/$LABEL.plist"
printf '\033]0;Universal Overlay — setup\007'

log() {
    printf '  • %s\n' "$*"
    printf '[%s] %s\n' "$(date '+%F %T')" "$*" >> "$LOG"
}
fail() {
    printf '  ✗ %s\n' "$*" >&2
    printf '[%s] ERROR: %s\n' "$(date '+%F %T')" "$*" >> "$LOG"
    [ $MODE = install ] && { printf '\n  Press Enter… '; read -r _; }
    exit 1
}
# Use administrator rights only when the folder is not writable without them.
as_needed() {
    local parent
    parent=$(dirname "$DEST")
    if [ -w "$parent" ] && { [ ! -e "$DEST" ] || [ -w "$DEST" ]; }; then
        "$@"
    else
        if [ -z "${ASKED:-}" ]; then
            echo "  The administrator password is needed to write to $parent"
            ASKED=1
        fi
        sudo "$@"
    fi
}

# ---------------------------------------------------------------- source
if [ -d "$ROOT/app/$APPNAME" ]; then
    SRC_APP="$ROOT/app/$APPNAME"
elif [ -d "$ROOT/$APPNAME" ]; then
    SRC_APP="$ROOT/$APPNAME"          # DMG
elif [ -f "$ROOT/source/Cargo.toml" ]; then
    command -v cargo >/dev/null 2>&1 || fail "Rust is required: https://rustup.rs (or download a release)"
    echo "Building Universal Overlay from source…"
    PKG=$(sh "$ROOT/source/build/build.sh" --print-package | tail -n 1) || fail "the build failed"
    exec /bin/bash "$PKG/install.command" "$@"
else
    fail "$APPNAME not found next to this script"
fi
VERSION=$(cat "$SRC_APP/Contents/Resources/VERSION" 2>/dev/null || echo "?")

clear 2>/dev/null
echo
echo "  Universal Overlay $VERSION — $( [ $MODE = update ] && echo update || echo setup )"
echo "  ─────────────────────────────────────────"
echo

# ---------------------------------------------------------------- DMG: drag and drop
if [ $MODE = install ] && [ -L "$ROOT/Applications" ]; then
    echo "  Drag \"Universal Overlay\" onto the \"Applications\" folder in the disk window."
    echo "  Or press Enter and it will be copied for you."
    echo
    open "$ROOT" 2>/dev/null
    had=0
    [ -d "$DEST" ] && had=1 && echo "  (The app is already installed — Enter replaces it with the new version.)"
    while :; do
        if read -r -t 1 _; then break; fi
        if [ $had = 0 ] && [ -f "$DEST/Contents/Info.plist" ]; then
            echo "  • \"Universal Overlay\" appeared in Applications, waiting for the copy to finish…"
            p=-1
            while :; do
                c=$(du -sk "$DEST" 2>/dev/null | cut -f1)
                [ "$c" = "$p" ] && break
                p=$c
                sleep 1
            done
            SRC_APP="$DEST"           # already copied by the user
            break
        fi
    done
fi

[ $MODE = update ] && log "Updating to $VERSION" || log "Installing $VERSION"

# ---------------------------------------------------------------- 1. stop
log "Stopping the running overlay"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null
if [ -x "$DEST/Contents/MacOS/universal-overlay" ]; then
    "$DEST/Contents/MacOS/universal-overlay" --quit 2>/dev/null
fi
sleep 0.5
# Match the full command line: macOS cuts process names to 16 characters,
# so pkill -x would never find them.
pkill -f "/universal-overlay-settings( |$)" 2>/dev/null
pkill -f "/MacOS/universal-overlay( |$)" 2>/dev/null

# ---------------------------------------------------------------- 2. files
if [ "$SRC_APP" != "$DEST" ]; then
    log "Removing the previous version and copying the new one"
    if [ -e "$DEST" ]; then as_needed rm -rf "$DEST" || fail "could not remove the old version"; fi
    as_needed ditto "$SRC_APP" "$DEST" || fail "could not copy the app"
else
    log "The app is already in $(dirname "$DEST")"
fi
# An app downloaded by a browser is quarantined — Gatekeeper would ask about
# every helper program. Remove the flag.
log "Removing the quarantine flag"
as_needed xattr -dr com.apple.quarantine "$DEST" 2>/dev/null

# ---------------------------------------------------------------- 3. autostart
log "Enabling autostart at login"
mkdir -p "$(dirname "$AGENT")"
cat > "$AGENT" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key><string>$LABEL</string>
    <key>ProgramArguments</key>
    <array>
        <string>$DEST/Contents/MacOS/universal-overlay</string>
        <string>--background</string>
    </array>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><false/>
    <!-- Do not kill child processes (settings, updater) together with the
         overlay: the updater stops the overlay in the middle of its work. -->
    <key>AbandonProcessGroup</key><true/>
    <key>LimitLoadToSessionType</key><string>Aqua</string>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
EOF

# ---------------------------------------------------------------- 4. start
log "Starting the overlay"
launchctl bootstrap "gui/$(id -u)" "$AGENT" 2>/dev/null || open "$DEST"

echo
echo "  Done. The Universal Overlay icon is in the menu bar; Ctrl+Option+O shows/hides it."
if [ $MODE = install ]; then
    echo "  For FPS, macOS will ask for the Screen Recording permission — please allow it"
    echo "  (System Settings → Privacy & Security → Screen Recording)."
    echo "  Log: $LOG"
    echo
    echo "  You can close this window."
fi
printf '[%s] Done\n' "$(date '+%F %T')" >> "$LOG"
exit 0
