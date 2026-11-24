#!/bin/sh
# Universal Overlay — сборка релиза для macOS (Apple Silicon + Intel).
#
# Результат (всё внутри source/target, корень репозитория не трогается):
#   target/package/universal-overlay-macos-universal/
#       install.command  update.command  uninstall.command   <- наверху только они
#       app/Universal Overlay.app
#   target/dist/universal-overlay-macos-universal.tar.gz   (его скачивает апдейтер)
#   target/dist/universal-overlay-macos.dmg                (для людей: окно «перетащите
#                                                           в Applications» + журнал установки)
#   target/dist/SHA256SUMS-macos.txt
#
#   --print-package   последней строкой вывести путь к папке пакета
# Переменные:
#   UO_GITHUB_REPO=owner/name   откуда апдейтер берёт релизы
#   MACOS_SIGN_IDENTITY         сертификат для codesign (иначе ad-hoc подпись)
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD   нотаризация (нужен Developer ID)
set -eu

SOURCE=$(cd "$(dirname "$0")/.." && pwd)
REPO=$(dirname "$SOURCE")
cd "$SOURCE"
VERSION=$(sed -n 's/^version *= *"\(.*\)"/\1/p' Cargo.toml | head -n 1)
NAME=universal-overlay-macos-universal
PKG=$SOURCE/target/package/$NAME
APP="$PKG/app/Universal Overlay.app"
DIST=$SOURCE/target/dist
BINS="universal-overlay universal-overlay-settings universal-overlay-updater uo-probe"
echo "== Universal Overlay $VERSION (macOS)" >&2
[ -n "${UO_GITHUB_REPO:-}" ] || echo "   UO_GITHUB_REPO не задан: апдейтер использует wizzard111/universal-overlay-macos" >&2

for t in aarch64-apple-darwin x86_64-apple-darwin; do
    rustup target add "$t" >/dev/null 2>&1 || true
    cargo build --release --target "$t" --workspace >&2
done

rm -rf "$PKG"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" "$DIST"
for b in $BINS; do
    lipo -create -output "$APP/Contents/MacOS/$b" \
        "target/aarch64-apple-darwin/release/$b" "target/x86_64-apple-darwin/release/$b"
done
sed "s/@VERSION@/$VERSION/g" resources/Info.plist > "$APP/Contents/Info.plist"
printf 'APPL????' > "$APP/Contents/PkgInfo"
printf '%s\n' "$VERSION" > "$APP/Contents/Resources/VERSION"
cp "$REPO/update.command" "$REPO/uninstall.command" "$REPO/docs/README.md" "$APP/Contents/Resources/"
chmod 755 "$APP/Contents/Resources/"*.command

# Иконка: рисует сам оверлей (сжатые PNG), iconutil собирает .icns.
ICONSET=$(mktemp -d)/icon.iconset
mkdir -p "$ICONSET"
for s in 16 32 128 256 512; do
    "$APP/Contents/MacOS/universal-overlay" --export-icon "$ICONSET/icon_${s}x${s}.png" "$s"
    "$APP/Contents/MacOS/universal-overlay" --export-icon "$ICONSET/icon_${s}x${s}@2x.png" $((s * 2))
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/icon.icns"

# Подпись. С постоянным сертификатом macOS помнит разрешение «Запись экрана»
# между обновлениями; при ad-hoc подписи его придётся выдавать заново.
IDENT=${MACOS_SIGN_IDENTITY:--}
if [ "$IDENT" = "-" ]; then SIGN_OPTS=""; else SIGN_OPTS="--options runtime --timestamp"; fi
for b in universal-overlay-settings universal-overlay-updater uo-probe; do
    # shellcheck disable=SC2086
    codesign --force $SIGN_OPTS --sign "$IDENT" --identifier "com.universaloverlay.$b" "$APP/Contents/MacOS/$b" >&2
done
# shellcheck disable=SC2086
codesign --force $SIGN_OPTS --sign "$IDENT" --identifier com.universaloverlay.app "$APP" >&2
codesign --verify --deep --strict "$APP" >&2

notarize() {
    if [ "$IDENT" != "-" ] && [ -n "${APPLE_ID:-}" ] && [ -n "${APPLE_TEAM_ID:-}" ] && [ -n "${APPLE_APP_PASSWORD:-}" ]; then
        xcrun notarytool submit "$1" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_APP_PASSWORD" --wait >&2
        return 0
    fi
    return 1
}
ZIP=$(mktemp -d)/app.zip
ditto -c -k --keepParent "$APP" "$ZIP"
if notarize "$ZIP"; then xcrun stapler staple "$APP" >&2; fi

# Архив для апдейтера.
cp "$REPO/install.command" "$REPO/update.command" "$REPO/uninstall.command" "$PKG/"
chmod 755 "$PKG"/*.command
tar -C "$SOURCE/target/package" -czf "$DIST/$NAME.tar.gz" "$NAME"

# DMG: приложение со стрелкой на «Applications», рядом установка и удаление.
DMG="$DIST/universal-overlay-macos.dmg"
rm -f "$DMG"
STAGE="$(mktemp -d)/Universal Overlay"
mkdir -p "$STAGE"
ditto "$APP" "$STAGE/Universal Overlay.app"
cp "$REPO/install.command" "$STAGE/Установить.command"
cp "$REPO/uninstall.command" "$STAGE/Удалить.command"
chmod 755 "$STAGE"/*.command
if command -v create-dmg >/dev/null 2>&1; then
    create-dmg --volname "Universal Overlay" --window-pos 200 120 --window-size 660 420 --icon-size 96 \
        --icon "Universal Overlay.app" 170 170 --app-drop-link 490 170 \
        --icon "Установить.command" 170 320 --icon "Удалить.command" 490 320 \
        --hide-extension "Установить.command" --hide-extension "Удалить.command" \
        --no-internet-enable "$DMG" "$STAGE" >&2 || true
fi
if [ ! -f "$DMG" ]; then
    ln -s /Applications "$STAGE/Applications"
    hdiutil create -volname "Universal Overlay" -srcfolder "$STAGE" -ov -format UDZO "$DMG" >&2
fi
if [ "$IDENT" != "-" ]; then
    codesign --force --sign "$IDENT" --timestamp "$DMG" >&2
    if notarize "$DMG"; then xcrun stapler staple "$DMG" >&2; fi
fi
(cd "$DIST" && shasum -a 256 "$NAME.tar.gz" "$(basename "$DMG")" > SHA256SUMS-macos.txt)
echo "== Готово: $DIST" >&2
if [ "${1:-}" = --print-package ]; then echo "$PKG"; fi
