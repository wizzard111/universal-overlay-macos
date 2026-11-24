#!/bin/bash
# Universal Overlay — обновление (macOS). Окно Терминала — журнал.
#
# В клоне репозитория с git: забирает изменения, пересобирает и ставит в
# режиме обновления. Иначе запускает апдейтер: он скачивает последний релиз
# с GitHub, проверяет SHA-256 и обновляет приложение (настройки остаются).
set -u
printf '\033]0;Universal Overlay — обновление\007'
ROOT=$(cd "$(dirname "$0")" && pwd)
APP="/Applications/Universal Overlay.app"

if [ -d "$ROOT/.git" ] && [ -f "$ROOT/source/Cargo.toml" ] && command -v git >/dev/null && command -v cargo >/dev/null; then
    echo "Получаю последние изменения…"
    git -C "$ROOT" pull --ff-only || exit 1
    PKG=$(sh "$ROOT/source/build/build.sh" --print-package | tail -n 1) || exit 1
    exec /bin/bash "$PKG/install.command" --update
fi

# Внутри .app (Contents/Resources) или рядом с ним.
for u in "$ROOT/../MacOS/universal-overlay-updater" "$ROOT/app/Universal Overlay.app/Contents/MacOS/universal-overlay-updater" \
         "$APP/Contents/MacOS/universal-overlay-updater"; do
    if [ -x "$u" ]; then
        "$u" --install-dir "$APP"
        exit $?
    fi
done
echo "Universal Overlay не установлен. Откройте install.command."
exit 1
