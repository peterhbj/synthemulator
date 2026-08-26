#!/usr/bin/env bash
# Install Helix as a user desktop app (~/.local), no root needed.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN_DIR="${XDG_BIN_HOME:-$HOME/.local/bin}"
APP_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICON_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
ICON_BASE="$ICON_DIR/256x256/apps/helix-synth.png"
ICON_SVG="$ICON_DIR/scalable/apps/helix-synth.svg"
TARGET_BIN="$BIN_DIR/helix-synth"

# Prefer an already-built release; otherwise build.
CANDIDATES=(
  "${CARGO_TARGET_DIR:-/tmp/helix-target}/release/helix"
  "$ROOT/target/release/helix"
  "${CARGO_TARGET_DIR:-/tmp/helix-target}/release/helix-synth"
)
SRC_BIN=""
for c in "${CANDIDATES[@]}"; do
  if [[ -x "$c" ]]; then
    SRC_BIN="$c"
    break
  fi
done

if [[ -z "$SRC_BIN" ]]; then
  echo "Building release (this can take a few minutes)…"
  export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/helix-target}"
  cargo build --release --manifest-path "$ROOT/Cargo.toml"
  SRC_BIN="$CARGO_TARGET_DIR/release/helix"
fi

mkdir -p "$BIN_DIR" "$APP_DIR" "$(dirname "$ICON_BASE")" "$(dirname "$ICON_SVG")"
install -m 755 "$SRC_BIN" "$TARGET_BIN"

cp "$ROOT/assets/helix-synth.svg" "$ICON_SVG"
if command -v rsvg-convert >/dev/null 2>&1; then
  rsvg-convert -w 256 -h 256 "$ROOT/assets/helix-synth.svg" -o "$ICON_BASE"
elif command -v magick >/dev/null 2>&1; then
  magick -background none "$ROOT/assets/helix-synth.svg" -resize 256x256 "$ICON_BASE"
else
  cp "$ICON_SVG" "$APP_DIR/helix-synth.svg"
  ICON_BASE="$ICON_SVG"
fi

sed -e "s|@BIN@|$TARGET_BIN|g" -e "s|@ICON@|$ICON_BASE|g" \
  "$ROOT/packaging/helix-synth.desktop.in" > "$APP_DIR/helix-synth.desktop"
chmod 644 "$APP_DIR/helix-synth.desktop"

if command -v update-desktop-database >/dev/null 2>&1; then
  update-desktop-database "$APP_DIR" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
  gtk-update-icon-cache -f -t "$ICON_DIR" >/dev/null 2>&1 || true
fi
if command -v xdg-desktop-menu >/dev/null 2>&1; then
  xdg-desktop-menu forceupdate >/dev/null 2>&1 || true
fi

echo "Installed Helix as a desktop app."
echo "  binary : $TARGET_BIN"
echo "  menu   : $APP_DIR/helix-synth.desktop"
echo "Search for “Helix” in the app launcher, or run: helix-synth"
