#!/bin/bash
# Universal Overlay — установка и обновление (macOS).
#
# Двойной щелчок открывает Терминал — это окно и есть журнал установки.
#   С диска DMG: рядом открыто окно диска, перетащите «Universal Overlay» на
#                «Applications» (или нажмите Enter — скопирую сам).
#   Из архива:   ставит app/Universal Overlay.app.
#   Из клона репозитория: сначала собирает программу (нужен Rust и Xcode CLT).
#
#   --update            обновление без вопросов (так вызывает апдейтер)
#   --install-dir PATH  другое место .app (по умолчанию /Applications)
#
# Пароль администратора спрашивается, только если в /Applications нельзя
# писать без него. Разрешение «Запись экрана» (для FPS) macOS спросит сама.
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
printf '\033]0;Universal Overlay — установка\007'

log() {
    printf '  • %s\n' "$*"
    printf '[%s] %s\n' "$(date '+%F %T')" "$*" >> "$LOG"
}
fail() {
    printf '  ✗ %s\n' "$*" >&2
    printf '[%s] ОШИБКА: %s\n' "$(date '+%F %T')" "$*" >> "$LOG"
    [ $MODE = install ] && { printf '\n  Нажмите Enter… '; read -r _; }
    exit 1
}
# С правами администратора — только если без них в папку не записать.
as_needed() {
    local parent
    parent=$(dirname "$DEST")
    if [ -w "$parent" ] && { [ ! -e "$DEST" ] || [ -w "$DEST" ]; }; then
        "$@"
    else
        if [ -z "${ASKED:-}" ]; then
            echo "  Нужен пароль администратора, чтобы записать в $parent"
            ASKED=1
        fi
        sudo "$@"
    fi
}

# ---------------------------------------------------------------- откуда ставим
if [ -d "$ROOT/app/$APPNAME" ]; then
    SRC_APP="$ROOT/app/$APPNAME"
elif [ -d "$ROOT/$APPNAME" ]; then
    SRC_APP="$ROOT/$APPNAME"          # диск DMG
elif [ -f "$ROOT/source/Cargo.toml" ]; then
    command -v cargo >/dev/null 2>&1 || fail "Нужен Rust: https://rustup.rs (или скачайте готовый релиз)"
    echo "Сборка Universal Overlay из исходников…"
    PKG=$(sh "$ROOT/source/build/build.sh" --print-package | tail -n 1) || fail "сборка не удалась"
    exec /bin/bash "$PKG/install.command" "$@"
else
    fail "не найдено $APPNAME рядом со скриптом"
fi
VERSION=$(cat "$SRC_APP/Contents/Resources/VERSION" 2>/dev/null || echo "?")

clear 2>/dev/null
echo
echo "  Universal Overlay $VERSION — $( [ $MODE = update ] && echo обновление || echo установка )"
echo "  ─────────────────────────────────────────"
echo

# ---------------------------------------------------------------- DMG: перетаскивание
if [ $MODE = install ] && [ -L "$ROOT/Applications" ]; then
    echo "  Перетащите «Universal Overlay» на папку «Applications» в окне диска."
    echo "  Или нажмите Enter — скопирую сам."
    echo
    open "$ROOT" 2>/dev/null
    had=0
    [ -d "$DEST" ] && had=1 && echo "  (Приложение уже установлено — Enter заменит его новой версией.)"
    while :; do
        if read -r -t 1 _; then break; fi
        if [ $had = 0 ] && [ -f "$DEST/Contents/Info.plist" ]; then
            echo "  • Вижу «Universal Overlay» в Applications, жду окончания копирования…"
            p=-1
            while :; do
                c=$(du -sk "$DEST" 2>/dev/null | cut -f1)
                [ "$c" = "$p" ] && break
                p=$c
                sleep 1
            done
            SRC_APP="$DEST"           # уже скопировано пользователем
            break
        fi
    done
fi

[ $MODE = update ] && log "Обновление до $VERSION" || log "Установка $VERSION"

# ---------------------------------------------------------------- 1. остановка
log "Останавливаю запущенный оверлей"
launchctl bootout "gui/$(id -u)/$LABEL" 2>/dev/null
if [ -x "$DEST/Contents/MacOS/universal-overlay" ]; then
    "$DEST/Contents/MacOS/universal-overlay" --quit 2>/dev/null
fi
sleep 0.5
# Процессы ищем по полной командной строке: имя процесса в macOS обрезается
# до 16 символов, и pkill -x его не находит.
pkill -f "/universal-overlay-settings( |$)" 2>/dev/null
pkill -f "/MacOS/universal-overlay( |$)" 2>/dev/null

# ---------------------------------------------------------------- 2. файлы
if [ "$SRC_APP" != "$DEST" ]; then
    log "Удаляю предыдущую версию и копирую новую"
    if [ -e "$DEST" ]; then as_needed rm -rf "$DEST" || fail "не удалось удалить старую версию"; fi
    as_needed ditto "$SRC_APP" "$DEST" || fail "не удалось скопировать приложение"
else
    log "Приложение уже в $(dirname "$DEST")"
fi
# Скачанное браузером приложение помечено карантином — Gatekeeper спрашивал
# бы про каждую вспомогательную программу. Снимаем метку.
log "Снимаю метку карантина"
as_needed xattr -dr com.apple.quarantine "$DEST" 2>/dev/null

# ---------------------------------------------------------------- 3. автозапуск
log "Включаю автозапуск при входе"
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
    <!-- Не убивать дочерние процессы (настройки, апдейтер) вместе с оверлеем:
         апдейтер останавливает оверлей посреди своей работы. -->
    <key>AbandonProcessGroup</key><true/>
    <key>LimitLoadToSessionType</key><string>Aqua</string>
    <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
EOF

# ---------------------------------------------------------------- 4. запуск
log "Запускаю оверлей"
launchctl bootstrap "gui/$(id -u)" "$AGENT" 2>/dev/null || open "$DEST"

echo
echo "  Готово. Значок Universal Overlay — в строке меню; Ctrl+Alt+O — показать/скрыть."
if [ $MODE = install ]; then
    echo "  Для FPS macOS попросит разрешение «Запись экрана» — разрешите его"
    echo "  (Системные настройки → Конфиденциальность и безопасность → Запись экрана)."
    echo "  Журнал: $LOG"
    echo
    echo "  Это окно можно закрыть."
fi
printf '[%s] Готово\n' "$(date '+%F %T')" >> "$LOG"
exit 0
