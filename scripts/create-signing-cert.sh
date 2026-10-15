#!/usr/bin/env bash
# Create a self-signed code-signing certificate for local Handy NB builds.
#
# Signing every build with the same certificate gives the app a stable
# identity, so macOS keeps its Accessibility permission across rebuilds.
# Run once. You will be asked for your login password to trust the
# certificate for code signing.
set -euo pipefail

NAME="${HANDY_NB_SIGNING_NAME:-Handy NB Local Signing}"
DAYS=3650
BACKUP_DIR="$HOME/.config/handy-nb/signing"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
# macOS LibreSSL: its PKCS#12 output is readable by `security import`
# (Homebrew OpenSSL 3 defaults are not).
OPENSSL=/usr/bin/openssl

if security find-certificate -c "$NAME" "$KEYCHAIN" >/dev/null 2>&1; then
  echo "A certificate named \"$NAME\" already exists in your login keychain."
  security find-identity -p codesigning "$KEYCHAIN" || true
  echo "Nothing to do. Delete it in Keychain Access first if you want a new one."
  exit 0
fi

mkdir -p "$BACKUP_DIR"
chmod 700 "$BACKUP_DIR"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/cert.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $NAME
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF

echo "Creating certificate \"$NAME\" (valid for $DAYS days)..."
"$OPENSSL" req -x509 -newkey rsa:2048 -nodes -sha256 -days "$DAYS" \
  -config "$WORK/cert.cnf" -keyout "$WORK/key.pem" -out "$WORK/cert.pem" 2>/dev/null

P12_PASSWORD="$("$OPENSSL" rand -hex 24)"
"$OPENSSL" pkcs12 -export -name "$NAME" -inkey "$WORK/key.pem" -in "$WORK/cert.pem" \
  -out "$WORK/identity.p12" -passout "pass:$P12_PASSWORD"

echo "Importing into your login keychain (codesign is allowed to use the key)..."
security import "$WORK/identity.p12" -k "$KEYCHAIN" -P "$P12_PASSWORD" \
  -T /usr/bin/codesign -T /usr/bin/security >/dev/null

echo "Trusting the certificate for code signing (macOS will ask for your password)..."
security add-trusted-cert -r trustRoot -p codeSign -k "$KEYCHAIN" "$WORK/cert.pem"

# Backup so the same identity can be restored later (e.g. on a new Mac).
cp "$WORK/identity.p12" "$BACKUP_DIR/handy-nb-signing.p12"
printf '%s\n' "$P12_PASSWORD" > "$BACKUP_DIR/handy-nb-signing.p12.password"
chmod 600 "$BACKUP_DIR"/*

echo
IDENTITIES="$(security find-identity -v -p codesigning "$KEYCHAIN" || true)"
if [[ "$IDENTITIES" == *"$NAME"* ]]; then
  echo "Code-signing identity ready: $NAME"
else
  echo "Warning: \"$NAME\" is not listed as a valid code-signing identity." >&2
  echo "Open Keychain Access and set the certificate to \"Always Trust\" for Code Signing." >&2
fi
echo
echo "Done. Backup: $BACKUP_DIR (keep it safe, never commit it)."
echo "Build with: scripts/build-nb.sh"
