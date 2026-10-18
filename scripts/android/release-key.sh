#!/bin/bash
# Creates the Android release signing key ONCE: DIR/key.pem (EC P-256,
# unencrypted PKCS#8, mode 600) and DIR/cert.der (self-signed, CN=baba-port,
# valid 10000 days), the format scripts/android/apk.py signs with.
#
# Keep DIR outside git (and outside tools/, which is disposable), back it up
# offline (password manager / encrypted USB stick). Every update of the app
# must be signed with this key: Android refuses an update signed by another
# key, so losing it means every player has to uninstall (and lose their
# saves) to update. For the release workflow, store both files as GitHub
# Actions secrets (base64), see docs/platform-android.md "Release signing".
#
# Usage: scripts/android/release-key.sh DIR
set -euo pipefail
DIR=${1:?usage: $0 DIR}
if [ -e "$DIR/key.pem" ] || [ -e "$DIR/cert.der" ]; then
    echo "$DIR already has a key: refusing to overwrite it" >&2
    exit 1
fi
mkdir -p "$DIR"
chmod 700 "$DIR"
( umask 077
  openssl genpkey -algorithm EC -pkeyopt ec_paramgen_curve:P-256 -out "$DIR/key.pem" )
openssl req -x509 -new -key "$DIR/key.pem" -subj "/CN=baba-port" -days 10000 \
    -outform DER -out "$DIR/cert.der"
echo "release key: $DIR/key.pem, certificate: $DIR/cert.der"
echo "certificate SHA-256 (publish it so players can check the APK):"
openssl x509 -inform DER -in "$DIR/cert.der" -noout -fingerprint -sha256
cat <<EOF

Next:
  - back up $DIR offline now;
  - local release builds: BABA_ANDROID_KEY_DIR=$DIR scripts/package.sh
  - CI (release.yml): repository secrets
      ANDROID_RELEASE_KEY_PEM_B64  = output of: base64 -w0 $DIR/key.pem
      ANDROID_RELEASE_CERT_DER_B64 = output of: base64 -w0 $DIR/cert.der
EOF
