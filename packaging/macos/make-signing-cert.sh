#!/usr/bin/env bash
# Creates BenCode's self-signed code signing certificate, once, for the
# release workflow's MACOS_CERTIFICATE secret (docs/releasing.md).
#
#   packaging/macos/make-signing-cert.sh [out-dir]
#
# macOS keeps the permissions a user grants (Screen Recording, Accessibility,
# Keychain items…) against the app's designated requirement. Ad hoc, that is
# the binary's hash and every update loses them; signed with this
# certificate, it names the certificate, which every release shares.
#
# Writes BenCode-signing.p12 and its password into out-dir (default: a new
# temporary folder). Keep both in a password manager: a new certificate makes
# every user grant their permissions again.
set -euo pipefail

name="BenCode Self-Signed"
out="${1:-$(mktemp -d)}"
mkdir -p "$out"
p12="$out/BenCode-signing.p12"
if [[ -e "$p12" ]]; then
  echo "$p12 already exists; not replacing it." >&2
  exit 1
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
password="$(/usr/bin/openssl rand -hex 24)"

cat >"$work/cert.conf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
subjectKeyIdentifier = hash
EOF

# 20 years: codesign refuses an expired certificate, and a new one costs
# every user their permissions.
/usr/bin/openssl req -x509 -newkey rsa:3072 -nodes -days 7300 -sha256 \
  -config "$work/cert.conf" -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
# /usr/bin/openssl is LibreSSL, whose PKCS#12 defaults `security import` reads
# (OpenSSL 3's AES ones it does not).
/usr/bin/openssl pkcs12 -export -name "$name" -inkey "$work/key.pem" -in "$work/cert.pem" \
  -passout "pass:$password" -out "$p12"
printf '%s\n' "$password" >"$out/BenCode-signing.password"
chmod 600 "$p12" "$out/BenCode-signing.password"

cat <<EOF
Created $p12
SHA-1: $(/usr/bin/openssl x509 -in "$work/cert.pem" -noout -fingerprint -sha1 | cut -d= -f2)

Add two secrets under repo › Settings › Secrets and variables › Actions:
  MACOS_CERTIFICATE           base64 -i "$p12" | pbcopy
  MACOS_CERTIFICATE_PASSWORD  pbcopy < "$out/BenCode-signing.password"

Then store both files in a password manager and delete $out.
EOF
