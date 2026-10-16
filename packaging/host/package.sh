#!/usr/bin/env bash
# BenCode Host's release packages (MonoCode `host/package.mjs`): one
# `bencode-host-<os>-<arch>.tar.gz` per platform, each holding the static
# `bencode-host` binary, and its `.sha256`, in target/bundle. Settings ›
# Connections downloads the one for a machine from this version's release
# and checks it against the checksum (src/remote/bootstrap.sh).
#
# macOS builds natively; Linux cross-builds against musl with
# cargo-zigbuild (`brew install zig && cargo install cargo-zigbuild`), so
# the binary runs on any distribution. HOST_TARGETS picks the platforms.
set -euo pipefail

cd "$(dirname "$0")/../.."
out=target/bundle
mkdir -p "$out"
targets=${HOST_TARGETS:-"aarch64-apple-darwin x86_64-apple-darwin aarch64-unknown-linux-musl x86_64-unknown-linux-musl"}
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n 1)"

for target in $targets; do
  case "$target" in
    aarch64-apple-darwin) name=darwin-arm64 ;;
    x86_64-apple-darwin) name=darwin-x64 ;;
    aarch64-unknown-linux-musl) name=linux-arm64 ;;
    x86_64-unknown-linux-musl) name=linux-x64 ;;
    *) echo "unknown target $target" >&2; exit 1 ;;
  esac
  echo "==> bencode-host $version for $name"
  case "$target" in
    *-apple-darwin) cargo build --release --locked -p bencode-host --target "$target" ;;
    *) cargo zigbuild --release --locked -p bencode-host --target "$target" ;;
  esac
  binary="target/$target/release/bencode-host"
  if [[ "$target" == *-apple-darwin && -n "${MACOS_SIGNING_IDENTITY:-}" ]]; then
    codesign --force --options runtime --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$binary"
  fi
  stage="$(mktemp -d)"
  cp "$binary" "$stage/bencode-host"
  chmod 755 "$stage/bencode-host"
  archive="bencode-host-$name.tar.gz"
  tar -czf "$out/$archive" -C "$stage" bencode-host
  rm -rf "$stage"
  (cd "$out" && shasum -a 256 "$archive" >"$archive.sha256")
done
