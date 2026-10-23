#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
APP_PATH="$ROOT_DIR/release/mac-arm64/Daily.app"
DMG_PATH="$ROOT_DIR/release/Daily-0.1.0-arm64.dmg"
STAGING_DIR="$(mktemp -d "${TMPDIR:-/tmp}/daily-dmg.XXXXXX")"
TEMP_DMG_PATH="${STAGING_DIR}.dmg"
HYBRID_DMG_PATH="${STAGING_DIR}.hybrid.dmg"

cleanup() {
  rm -rf "$STAGING_DIR"
  rm -f "$TEMP_DMG_PATH"
  rm -f "$HYBRID_DMG_PATH"
}
trap cleanup EXIT

if [[ ! -d "$APP_PATH" ]]; then
  echo "Packaged application was not found at $APP_PATH" >&2
  exit 1
fi

cp -R "$APP_PATH" "$STAGING_DIR/Daily.app"
ln -s /Applications "$STAGING_DIR/Applications"
if ! hdiutil create \
  -volname "Daily" \
  -srcfolder "$STAGING_DIR" \
  -ov \
  -format UDZO \
  "$TEMP_DMG_PATH"; then
  echo "Direct DMG creation failed; using a read-only hybrid image fallback." >&2
  rm -f "$TEMP_DMG_PATH"
  hdiutil makehybrid \
    -o "$HYBRID_DMG_PATH" \
    -hfs \
    -hfs-volume-name "Daily" \
    "$STAGING_DIR"
  hdiutil convert \
    "$HYBRID_DMG_PATH" \
    -format UDZO \
    -o "$TEMP_DMG_PATH"
fi

mv -f "$TEMP_DMG_PATH" "$DMG_PATH"
