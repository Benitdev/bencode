set -eu
umask 077
# MonoCode `remote_bootstrap.sh`: installs this desktop's BenCode Host,
# checked against its release's checksum, under ~/.bencode-host.
BASE="$HOME/.bencode-host"
VERSION=@@VERSION@@
RELEASE=@@RELEASE@@
UPGRADE=@@UPGRADE@@
DEST="$BASE/versions/$VERSION"
ENTRY="$BASE/bin/bencode-host"
mkdir -p "$BASE/bin" "$BASE/versions"
chmod 700 "$BASE"

case "$(uname -s)" in
  Darwin) OS=darwin ;;
  Linux) OS=linux ;;
  *) echo 'BenCode Host runs on Linux and macOS.' >&2; exit 1 ;;
esac
case "$(uname -m)" in
  arm64|aarch64) ARCH=arm64 ;;
  x86_64|amd64) ARCH=x64 ;;
  *) echo "BenCode Host does not run on $(uname -m) machines." >&2; exit 1 ;;
esac

installed() {
  [ -x "$DEST/bencode-host" ] && [ "$("$DEST/bencode-host" --version 2>/dev/null)" = "$VERSION" ]
}

if ! installed; then
  FILE="bencode-host-$OS-$ARCH.tar.gz"
  TMP=$(mktemp -d "$BASE/versions/.install.XXXXXXXX")
  trap 'rm -rf "$TMP"' EXIT
  trap 'exit 130' HUP INT TERM
  download() {
    if command -v curl >/dev/null 2>&1; then
      curl --proto '=https' --proto-redir '=https' -fLsS --connect-timeout 15 --max-time 300 "$1" -o "$2"
    elif command -v wget >/dev/null 2>&1; then
      wget --https-only --timeout=300 -q -O "$2" "$1"
    else
      echo 'Install curl or wget on this machine and reconnect.' >&2; exit 1
    fi
  }
  if ! download "$RELEASE/$FILE" "$TMP/$FILE" || ! download "$RELEASE/$FILE.sha256" "$TMP/checksum"; then
    echo "BenCode Host $VERSION for $OS-$ARCH is not published. Install a BenCode release that includes host packages." >&2; exit 1
  fi
  EXPECTED=$(awk 'NR == 1 {print $1}' "$TMP/checksum")
  case "$EXPECTED" in *[!0-9a-f]*|'') echo 'The host package has an invalid checksum.' >&2; exit 1 ;; esac
  [ "${#EXPECTED}" -eq 64 ] || { echo 'The host package has an invalid checksum.' >&2; exit 1; }
  if command -v shasum >/dev/null 2>&1; then
    ACTUAL=$(shasum -a 256 "$TMP/$FILE" | awk '{print $1}')
  elif command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$TMP/$FILE" | awk '{print $1}')
  else
    echo 'Install shasum or sha256sum on this machine and reconnect.' >&2; exit 1
  fi
  [ "$EXPECTED" = "$ACTUAL" ] || { echo 'The host package does not match its checksum.' >&2; exit 1; }
  mkdir "$TMP/unpacked"
  tar -xzf "$TMP/$FILE" -C "$TMP/unpacked"
  [ "$("$TMP/unpacked/bencode-host" --version)" = "$VERSION" ] || { echo 'The host package is the wrong version.' >&2; exit 1; }
  # A running host keeps its own folder: a new version goes beside it.
  rm -rf "$DEST"
  mv "$TMP/unpacked" "$DEST"
fi

ln -sfn "$DEST/bencode-host" "$ENTRY"

# systemd ends an account's processes when it logs out unless it lingers;
# the host keeps running agents after SSH closes.
if [ "$OS" = linux ] && command -v loginctl >/dev/null 2>&1; then
  if ! loginctl show-user "$(id -un)" -p Linger 2>/dev/null | grep -q 'Linger=yes'; then
    loginctl enable-linger "$(id -un)" >/dev/null 2>&1 || echo "BENCODE_LINGER_FAILED"
  fi
fi

if [ "$UPGRADE" = 1 ]; then
  "$ENTRY" stop >/dev/null 2>&1 || true
fi

# Older versions are kept until nothing runs them; drop all but the newest two.
ls -1t "$BASE/versions" 2>/dev/null | grep -v '^\.' | tail -n +3 | while read -r old; do
  [ "$old" = "$VERSION" ] || rm -rf "$BASE/versions/$old"
done

echo "BENCODE_HOST_READY $VERSION"
