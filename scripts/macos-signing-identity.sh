#!/usr/bin/env bash
# Print the persistent certificate fingerprint used by the local installer.
set -euo pipefail

if [[ $(uname -s) != Darwin ]]; then
  echo 'Local signing requires macOS.' >&2
  exit 1
fi

if [[ -n ${NEKOMATA_SIGNING_IDENTITY:-} ]]; then
  printf '%s\n' "$NEKOMATA_SIGNING_IDENTITY"
  exit 0
fi

signing_directory="${XDG_DATA_HOME:-$HOME/.local/share}/nekomata/signing"
certificate="$signing_directory/certificate.pem"
keychain=$(/usr/bin/security default-keychain -d user | sed -E 's/^[[:space:]]*"//; s/"[[:space:]]*$//')
umask 077
mkdir -p "$signing_directory"
chmod 700 "$signing_directory"
work=$(mktemp -d "$signing_directory/.setup.XXXXXX")
trap 'rm -rf "$work"' EXIT

if [[ ! -f "$certificate" ]]; then
  # Reuse an imported identity if only the public certificate file was removed.
  if ! /usr/bin/security find-certificate -c 'nekomata Local Code Signing' -p "$keychain" > "$work/certificate.pem" 2>/dev/null; then
    cat > "$work/openssl.cnf" <<'CONFIG'
[req]
prompt = no
distinguished_name = name
x509_extensions = signing
[name]
CN = nekomata Local Code Signing
[signing]
basicConstraints = critical,CA:FALSE
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
CONFIG
    echo 'Creating a persistent local nekomata code-signing certificate.' >&2
    /usr/bin/openssl req -new -x509 -newkey rsa:3072 -nodes -sha256 -days 3650 \
      -config "$work/openssl.cnf" -keyout "$work/key.pem" -out "$work/certificate.pem" 2> "$work/openssl.log"
    /usr/bin/openssl rand -hex 32 > "$work/password"
    /usr/bin/openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/certificate.pem" \
      -name 'nekomata Local Code Signing' -macalg sha1 \
      -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES \
      -passout "file:$work/password" -out "$work/identity.p12"
    # Keep the private key in the user's keychain, non-exportable, and authorize
    # codesign specifically rather than granting every application key access.
    /usr/bin/security import "$work/identity.p12" -k "$keychain" -P "$(< "$work/password")" -x -T /usr/bin/codesign >&2
  fi
  cp "$work/certificate.pem" "$certificate"
fi

identity=$(/usr/bin/openssl x509 -in "$certificate" -noout -fingerprint -sha1 | sed 's/.*=//; s/://g')
if ! /usr/bin/security find-identity -v -p codesigning "$keychain" | grep -F "$identity" >/dev/null; then
  # Trust this certificate only for code signing, in the current user's domain.
  # Do not grant TLS trust or modify the system keychain.
  /usr/bin/security add-trusted-cert -r trustRoot -p codeSign -k "$keychain" "$certificate"
fi
if ! /usr/bin/security find-identity -v -p codesigning "$keychain" | grep -F "$identity" >/dev/null; then
  echo "No usable private key for the signing certificate in $keychain" >&2
  exit 1
fi
printf '%s\n' "$identity"
