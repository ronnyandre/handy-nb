#!/usr/bin/env bash
# Build Handy NB, sign it with the local certificate and install it to
# /Applications. Usage: scripts/build-nb.sh [--no-install]
set -euo pipefail

NAME="${HANDY_NB_SIGNING_NAME:-Handy NB Local Signing}"
APP_NAME="Handy NB"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BUNDLE="$ROOT/src-tauri/target/release/bundle/macos/$APP_NAME.app"
DEST="/Applications/$APP_NAME.app"

IDENTITIES="$(security find-identity -v -p codesigning || true)"
if [[ "$IDENTITIES" != *"$NAME"* ]]; then
  echo "Signing certificate \"$NAME\" not found. Run scripts/create-signing-cert.sh first." >&2
  exit 1
fi

cd "$ROOT"
CONFIG=$(printf '{"bundle":{"createUpdaterArtifacts":false,"macOS":{"signingIdentity":"%s"}}}' "$NAME")
CMAKE_POLICY_VERSION_MINIMUM=3.5 bun run tauri build --bundles app --config "$CONFIG"

codesign --verify --strict "$BUNDLE"
echo "Designated requirement:"
codesign -d -r- "$BUNDLE" 2>&1 | sed -n 's/^designated => //p'

if [[ "${1:-}" == "--no-install" ]]; then
  echo "Built: $BUNDLE"
  exit 0
fi

osascript -e "quit app \"$APP_NAME\"" 2>/dev/null || true
for _ in 1 2 3 4 5; do
  pgrep -f "$DEST/Contents/MacOS/" >/dev/null || break
  sleep 1
done
pkill -f "$DEST/Contents/MacOS/" 2>/dev/null || true

rm -rf "$DEST"
ditto "$BUNDLE" "$DEST"
open "$DEST"
echo "Installed and started $DEST"
