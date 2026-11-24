#!/bin/bash
# Universal Overlay — uninstall (macOS). The Terminal window is the log.
#   --purge             remove your settings too, without asking
#   --install-dir PATH  another location of the .app
set -u

APPNAME="Universal Overlay.app"
LABEL=com.universaloverlay.app
DEST="/Applications/$APPNAME"
PURGE=0
prev=
for a in "$@"; do
    [ "$a" = --purge ] && PURGE=1
    [ "$prev" = --install-dir ] && DEST=$a
    prev=$a
done
printf '\033]0;Universal Overlay — uninstall\007'
log() { printf '  • %s\n' "$*"; }
as_needed() {
    if [ -w "$(dirname "$DEST")" ] && [ -w "$DEST" ]; then "$@"; else sudo "$@"; fi
}

echo
echo "  Uninstalling Universal Overlay"
echo
log "Stopping the overlay and removing autostart"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null
if [ -x "$DEST/Contents/MacOS/universal-overlay" ]; then "$DEST/Contents/MacOS/universal-overlay" --quit 2>/dev/null; fi
sleep 0.5
# Match the full command line: macOS cuts process names to 16 characters,
# so pkill -x would never find them.
pkill -f "/universal-overlay-settings( |$)" 2>/dev/null
pkill -f "/MacOS/universal-overlay( |$)" 2>/dev/null
rm -f "$HOME/Library/LaunchAgents/$LABEL.plist"
if [ -e "$DEST" ]; then
    log "Removing $DEST"
    as_needed rm -rf "$DEST"
fi
log "Resetting the Screen Recording permission"
tccutil reset ScreenCapture "$LABEL" >/dev/null 2>&1
CFG="$HOME/Library/Application Support/UniversalOverlay"
if [ -d "$CFG" ]; then
    if [ $PURGE = 0 ] && [ -t 0 ]; then
        printf '  Remove your settings too? [y/N] '
        read -r ans || ans=n
        case $ans in [yY]*) PURGE=1 ;; esac
    fi
    if [ $PURGE = 1 ]; then rm -rf "$CFG"; log "Settings removed"; else log "Settings kept: $CFG"; fi
fi
rm -rf "$HOME/Library/Logs/UniversalOverlay"
echo
echo "  Universal Overlay has been uninstalled. You can close this window."
