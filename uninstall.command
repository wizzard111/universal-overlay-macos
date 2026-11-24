#!/bin/bash
# Universal Overlay — удаление (macOS). Окно Терминала — журнал удаления.
#   --purge             удалить и настройки, без вопроса
#   --install-dir PATH  другое место .app
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
printf '\033]0;Universal Overlay — удаление\007'
log() { printf '  • %s\n' "$*"; }
as_needed() {
    if [ -w "$(dirname "$DEST")" ] && [ -w "$DEST" ]; then "$@"; else sudo "$@"; fi
}

echo
echo "  Удаление Universal Overlay"
echo
log "Останавливаю оверлей и убираю автозапуск"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null
if [ -x "$DEST/Contents/MacOS/universal-overlay" ]; then "$DEST/Contents/MacOS/universal-overlay" --quit 2>/dev/null; fi
sleep 0.5
# Процессы ищем по полной командной строке: имя процесса в macOS обрезается
# до 16 символов, и pkill -x его не находит.
pkill -f "/universal-overlay-settings( |$)" 2>/dev/null
pkill -f "/MacOS/universal-overlay( |$)" 2>/dev/null
rm -f "$HOME/Library/LaunchAgents/$LABEL.plist"
if [ -e "$DEST" ]; then
    log "Удаляю $DEST"
    as_needed rm -rf "$DEST"
fi
log "Сбрасываю разрешение «Запись экрана»"
tccutil reset ScreenCapture "$LABEL" >/dev/null 2>&1
CFG="$HOME/Library/Application Support/UniversalOverlay"
if [ -d "$CFG" ]; then
    if [ $PURGE = 0 ] && [ -t 0 ]; then
        printf '  Удалить и настройки? [y/N] '
        read -r ans || ans=n
        case $ans in [yYдД]*) PURGE=1 ;; esac
    fi
    if [ $PURGE = 1 ]; then rm -rf "$CFG"; log "Настройки удалены"; else log "Настройки оставлены: $CFG"; fi
fi
rm -rf "$HOME/Library/Logs/UniversalOverlay"
echo
echo "  Universal Overlay удалён. Это окно можно закрыть."
