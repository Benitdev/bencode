#!/usr/bin/env bash
# Builds BenCode.app and BenCode.dmg in target/bundle (macOS only).
#
#   packaging/macos/bundle.sh
#
# Environment (all optional):
#   TARGETS                 Rust targets to build; two or more are joined into
#                           one universal binary with lipo.
#                           Default: "aarch64-apple-darwin x86_64-apple-darwin".
#   BUILD_NUMBER            CFBundleVersion. Default: the commit count.
#   MACOS_SIGNING_IDENTITY  A "Developer ID Application: …" identity in the
#                           keychain. Unset, the app is signed ad hoc and
#                           Gatekeeper asks users to confirm the first launch.
#   APPLE_ID, APPLE_TEAM_ID, APPLE_APP_PASSWORD
#                           With a signing identity, the dmg is notarized and
#                           the ticket stapled to it.
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
here=packaging/macos
out=target/bundle
app="$out/BenCode.app"
dmg="$out/BenCode.dmg"

version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"
build="${BUILD_NUMBER:-$(git rev-list --count HEAD)}"
targets="${TARGETS:-aarch64-apple-darwin x86_64-apple-darwin}"
# Matches LSMinimumSystemVersion in Info.plist.
export MACOSX_DEPLOYMENT_TARGET=11.0

echo "==> BenCode $version ($build) for $targets"

binaries=()
for target in $targets; do
  cargo build --release --locked --target "$target"
  binaries+=("target/$target/release/bencode")
done

rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
if ((${#binaries[@]} > 1)); then
  lipo -create -output "$app/Contents/MacOS/bencode" "${binaries[@]}"
else
  cp "${binaries[0]}" "$app/Contents/MacOS/bencode"
fi
lipo -info "$app/Contents/MacOS/bencode"

sed -e "s/@VERSION@/$version/" -e "s/@BUILD@/$build/" "$here/Info.plist" >"$app/Contents/Info.plist"
plutil -lint "$app/Contents/Info.plist"
printf 'APPL????' >"$app/Contents/PkgInfo"

echo "==> Icon"
iconset="$out/AppIcon.iconset"
mkdir -p "$iconset"
for size in 16 32 128 256 512; do
  sips -z "$size" "$size" "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}.png" >/dev/null
  double=$((size * 2))
  sips -z "$double" "$double" "$here/icon-1024.png" --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$iconset" -o "$app/Contents/Resources/AppIcon.icns"
rm -rf "$iconset"

echo "==> Sign"
identity="${MACOS_SIGNING_IDENTITY:-}"
if [[ -n "$identity" ]]; then
  codesign --force --options runtime --timestamp \
    --entitlements "$here/entitlements.plist" --sign "$identity" "$app"
else
  echo "No MACOS_SIGNING_IDENTITY: signing ad hoc (users confirm the first launch)."
  codesign --force --sign - "$app"
fi
codesign --verify --strict --verbose=2 "$app"

echo "==> Disk image"
staging="$out/dmg"
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
    echo "==> Notarize"
    xcrun notarytool submit "$dmg" --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" \
      --password "$APPLE_APP_PASSWORD" --wait
    xcrun stapler staple "$dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "$dmg"
  else
    echo "No APPLE_ID / APPLE_TEAM_ID / APPLE_APP_PASSWORD: skipping notarization."
  fi
fi

(cd "$out" && shasum -a 256 BenCode.dmg >BenCode.dmg.sha256)
echo "==> $dmg"
cat "$out/BenCode.dmg.sha256"
