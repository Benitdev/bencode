#!/usr/bin/env bash
# Builds BenCode.app and a BenCode-<arch>.dmg for each architecture in
# target/bundle (macOS only).
#
#   packaging/macos/bundle.sh
#
# Environment (all optional):
#   TARGETS                 Rust targets to build; each gets its own app and
#                           disk image (BenCode-arm64.dmg, BenCode-x86_64.dmg).
#                           Default: "aarch64-apple-darwin x86_64-apple-darwin".
#   BUILD_NUMBER            CFBundleVersion. Default: the commit count.
#   MACOS_SIGNING_IDENTITY  A "Developer ID Application: …" identity in the
#                           keychain. Unset, the app is signed ad hoc and
#                           Gatekeeper asks users to confirm the first launch.
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD
#                           With a signing identity, each dmg is notarized and
#                           the ticket stapled to it.
#   BENCODE_UPDATE_PUBKEY   The release key's public half (minisign). Built
#                           into the app, which then updates itself.
#   MINISIGN_SECRET_KEY     The release key's secret half (a `minisign -G -W`
#                           key, no password). With it, each architecture's
#                           update archive BenCode-<arch>.app.tar.gz is signed
#                           and latest.json, the release feed, lists them.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
here=packaging/macos
out=target/bundle

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
build="${BUILD_NUMBER:-$(git rev-list --count HEAD)}"
targets="${TARGETS:-aarch64-apple-darwin x86_64-apple-darwin}"
identity="${MACOS_SIGNING_IDENTITY:-}"
# Matches LSMinimumSystemVersion in Info.plist.
export MACOSX_DEPLOYMENT_TARGET=11.0

# The name an architecture has in file names, as `uname -m` prints it.
arch_of() {
  case "$1" in
    aarch64-apple-darwin) echo arm64 ;;
    x86_64-apple-darwin) echo x86_64 ;;
    *)
      echo "Unknown target $1" >&2
      return 1
      ;;
  esac
}
for target in $targets; do
  arch_of "$target" >/dev/null
done

updates=false
if [[ -n "${MINISIGN_SECRET_KEY:-}" ]]; then
  if [[ -z "${BENCODE_UPDATE_PUBKEY:-}" ]]; then
    echo "MINISIGN_SECRET_KEY is set but BENCODE_UPDATE_PUBKEY is not: the app would not check the update." >&2
    exit 1
  fi
  updates=true
fi

echo "==> BenCode $version ($build) for $targets"

for target in $targets; do
  # The build runs every dependency's build script and proc macro: none of
  # them gets the release key or the notarization password.
  env -u MINISIGN_SECRET_KEY -u APPLE_ID -u APPLE_TEAM_ID -u APPLE_APP_PASSWORD \
    cargo build --release --locked --target "$target"
done

rm -rf "$out"
mkdir -p "$out"

echo "==> Icon"
iconset="$out/AppIcon.iconset"
icon="$out/AppIcon.icns"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$icon"
rm -rf "$iconset"

if [[ "$updates" == true ]]; then
  key_file="$(mktemp)"
  trap 'rm -f "$key_file"' EXIT
  printf '%s\n' "$MINISIGN_SECRET_KEY" >"$key_file"
  # The public key built in must check what the secret one signed. It may be
  # the base64 line alone or the whole .pub file.
  public_key="$(printf '%s\n' "$BENCODE_UPDATE_PUBKEY" | tr -d '\r' | grep -v '^untrusted comment:' | grep -v '^[[:space:]]*$' | tail -n 1 | tr -d '[:space:]')"
fi

# Each update archive's platform, file name, signature file and size.
feed=()
for target in $targets; do
  arch="$(arch_of "$target")"
  # Each architecture has its own folder: the app is BenCode.app in all of
  # them, which is the name the update archive unpacks to.
  app="$out/$arch/BenCode.app"
  dmg="$out/BenCode-$arch.dmg"

  echo "==> BenCode.app ($arch)"
  mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
  cp "target/$target/release/bencode" "$app/Contents/MacOS/bencode"
  lipo -info "$app/Contents/MacOS/bencode"
  sed -e "s/@VERSION@/$version/" -e "s/@BUILD@/$build/" "$here/Info.plist" >"$app/Contents/Info.plist"
  plutil -lint "$app/Contents/Info.plist"
  printf 'APPL????' >"$app/Contents/PkgInfo"
  cp "$icon" "$app/Contents/Resources/AppIcon.icns"

  echo "==> Sign ($arch)"
  if [[ -n "$identity" ]]; then
    codesign --force --options runtime --timestamp \
      --entitlements "$here/entitlements.plist" --sign "$identity" "$app"
  else
    echo "No MACOS_SIGNING_IDENTITY: signing ad hoc (users confirm the first launch)."
    codesign --force --sign - "$app"
  fi
  codesign --verify --strict --verbose=2 "$app"

  echo "==> Disk image ($arch)"
  staging="$out/$arch/dmg"
  mkdir -p "$staging"
  cp -R "$app" "$staging/"
  ln -s /Applications "$staging/Applications"
  # hdiutil fails now and then on CI runners ("Resource busy"); try again.
  for attempt in 1 2 3; do
    if hdiutil create -volname "BenCode" -srcfolder "$staging" -fs HFS+ -format UDZO -ov "$dmg"; then
      break
    fi
    if ((attempt == 3)); then
      echo "hdiutil failed three times" >&2
      exit 1
    fi
    sleep $((attempt * 5))
  done
  rm -rf "$staging"

  if [[ -n "$identity" ]]; then
    codesign --force --timestamp --sign "$identity" "$dmg"
    if [[ -n "${APPLE_ID:-}" && -n "${APPLE_TEAM_ID:-}" && -n "${APPLE_APP_PASSWORD:-}" ]]; then
      echo "==> Notarize ($arch)"
      xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
        --password "$APPLE_APP_PASSWORD" --wait
      xcrun stapler staple "$dmg"
      spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
    else
      echo "No APPLE_ID / APPLE_TEAM_ID / APPLE_APP_PASSWORD: skipping notarization."
    fi
  fi

  (cd "$out" && shasum -a 256 "BenCode-$arch.dmg" >"BenCode-$arch.dmg.sha256")
  echo "==> $dmg"
  cat "$dmg.sha256"

  if [[ "$updates" != true ]]; then
    continue
  fi
  echo "==> Update archive ($arch)"
  archive="$out/BenCode-$arch.app.tar.gz"
  # COPYFILE_DISABLE: no AppleDouble ._ files in the archive.
  (cd "$out/$arch" && COPYFILE_DISABLE=1 tar -czf "../BenCode-$arch.app.tar.gz" BenCode.app)
  minisign -S -s "$key_file" -m "$archive" -x "$archive.minisig" -t "BenCode $version ($arch)"
  minisign -V -P "$public_key" -m "$archive" -x "$archive.minisig"
  # Tauri's names for the two: darwin-aarch64, darwin-x86_64.
  feed+=("darwin-${target%%-*}" "BenCode-$arch.app.tar.gz" "$archive.minisig" "$(stat -f %z "$archive")")
done
rm -f "$icon"

if [[ "$updates" != true ]]; then
  echo "No MINISIGN_SECRET_KEY: no update archive or release feed."
  exit 0
fi
rm -f "$key_file"

echo "==> Release feed"
# Tauri's latest.json, with each build's size for the download's progress.
VERSION="$version" python3 - "${feed[@]}" >"$out/latest.json" <<'PY'
import datetime, json, os, sys

version = os.environ["VERSION"]
base = f"https://github.com/Benitdev/bencode/releases/download/v{version}/"
platforms = {}
for platform, name, signature_file, size in zip(*[iter(sys.argv[1:])] * 4):
    platforms[platform] = {
        "url": base + name,
        "signature": open(signature_file).read(),
        "size": int(size),
    }
print(json.dumps({
    "version": version,
    "notes": "BenCode " + version,
    "pub_date": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "platforms": platforms,
}, indent=2))
PY
cat "$out/latest.json"
