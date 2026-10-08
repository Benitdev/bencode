# Releasing BenCode

BenCode ships through **GitHub Releases** and a **landing page** on GitHub
Pages, not the App Store. Each release is one universal `BenCode.dmg` (Apple
Silicon and Intel, macOS 11 or later).

| Piece | Where |
| :--- | :--- |
| App icon (source: `icon.mjs`; generates the SVG and the 1024 PNG) | `packaging/macos/icon.mjs`, `icon.svg`, `icon-1024.png` |
| `Info.plist`, entitlements | `packaging/macos/` |
| `.app` / `.dmg` bundling script | `packaging/macos/bundle.sh` |
| Release workflow (`v*` tags) | `.github/workflows/release.yml` |
| Landing page | `site/index.html`, deployed by `.github/workflows/pages.yml` |
| Release notes | `CHANGELOG.md` |
| Self-update (app side) | `src/updater.rs`, `src/app/updater.rs`, `src/ui/rail/update.rs` |

---

## One-time setup

1. **Enable GitHub Pages:** repo › Settings › Pages › *Build and deployment* ›
   Source: **GitHub Actions**. Then run the **Pages** workflow (Actions › Pages
   › Run workflow), or push a change under `site/` to `main`. The page is
   served at `https://benitdev.github.io/bencode/`.
2. **The bundle ID** is `com.benitdev.bencode` (`packaging/macos/Info.plist`).
   If you want to change it, do so before the first release: macOS ties the
   permissions a user has granted (Documents, Desktop…) to the bundle ID.
3. **Update keys** let the app update itself, see "Self-update" below. Without
   them the app still runs; it just does not update itself.
4. **Signing and notarization** are optional, see the last section.

---

## Every release

1. Bump `version` in `Cargo.toml`, then run `cargo check` so `Cargo.lock`
   follows.
2. In `CHANGELOG.md`, change `## [x.y.z] - Unreleased` to the release date
   (`## [0.1.0] - 2026-10-10`) and list the changes. That section becomes the
   release notes; without it GitHub lists the commits instead.
3. Commit and merge into `main`.
4. Tag and push:

   ```bash
   git tag v0.1.0
   git push origin v0.1.0
   ```

5. Watch **Actions › Release**. The workflow builds both architectures with
   LTO, so it takes about 30 to 60 minutes. When it finishes, Release `v0.1.0`
   has `BenCode.dmg` and `BenCode.dmg.sha256`.
6. Test on another machine (or a fresh macOS user): download from the landing
   page, drag into Applications, launch for the first time, run a turn with an
   agent, open the terminal, Help › Show Logs.

The workflow stops immediately if the tag does not match the version in
`Cargo.toml`.

**Pre-releases:** a tag with a hyphen (`v0.2.0-beta.1`) is marked as a
pre-release. The landing page's `releases/latest/download/BenCode.dmg` link
skips pre-releases, so it keeps pointing at the latest stable build.

**Trial build without releasing:** Actions › Release › Run workflow. By default
a trial run builds Apple Silicon only and turns on *Quick build* (thin LTO,
parallel codegen), so it finishes much sooner; select both architectures and
turn *Quick build* off to build exactly like a release. The dmg is under that
run's *Artifacts*.

**Building locally:**

```bash
rustup target add aarch64-apple-darwin x86_64-apple-darwin
packaging/macos/bundle.sh                              # universal
TARGETS=aarch64-apple-darwin packaging/macos/bundle.sh # Apple Silicon only, faster
open target/bundle/BenCode.dmg
```

**Changing the icon:** the icon ("Ember Bronze") is drawn in code, in
`packaging/macos/icon.mjs`. Edit that file, then run:

```bash
NODE_PATH="$(npm root -g)" node packaging/macos/icon.mjs   # needs playwright
```

This rewrites `icon.svg` (also used by the landing page), `icon-1024.png` (from
which `bundle.sh` builds `AppIcon.icns`) and `assets/app-icon.png` (the
"Updated to" card on the rail). Do not edit those three files by hand.

---

## Self-update

The app checks for a new version once at launch, and whenever you choose
**BenCode › Check for Updates…** or Settings › About. When one is available the
rail shows an "Update to X" button. Clicking it makes the app:

1. download `BenCode.app.tar.gz` from the latest Release;
2. verify its minisign signature against the public key embedded at build time;
3. check the bundle ID, the version and `codesign --verify`;
4. replace the running `BenCode.app` with the new one and relaunch.

On the next launch the rail shows an "Updated to X / What's new" card. The
notes come from the `CHANGELOG.md` embedded in the app.

The app reads `https://github.com/Benitdev/bencode/releases/latest/download/latest.json`
(Tauri's `latest.json` format). `bundle.sh` generates that file together with
the archive and its signature; the Release workflow uploads all three to every
Release.

**Generating the keys (once, on your machine):**

```bash
brew install minisign
minisign -G -W -p bencode-update.pub -s bencode-update.key   # -W: no password
```

Then go to repo › Settings › Secrets and variables › Actions:

| Kind | Name | Value |
| :--- | :--- | :--- |
| Variable | `BENCODE_UPDATE_PUBKEY` | the second line of `bencode-update.pub` (the string starting with `RW`) |
| Secret | `MINISIGN_SECRET_KEY` | the entire contents of `bencode-update.key` |

Set both or neither; with only one of them the workflow stops with an error.
Keep `bencode-update.key` somewhere safe (a password manager) and never commit
it. If the key is lost, installed copies can no longer receive updates and
users have to download the new version by hand.

**Notes:**

- Only builds made with `BENCODE_UPDATE_PUBKEY` update themselves. `cargo run`
  has no key: "Check for Updates…" says this build does not self-update and
  links to Releases.
- The app has to live somewhere writable (usually `/Applications`). If macOS is
  running BenCode from a temporary copy (App Translocation, when it is opened
  straight from Downloads) or from the dmg, the app asks you to drag it into
  Applications first.
- A download made with `curl` carries no quarantine flag, so an update never
  asks for "Open Anyway" again.

---

## Without an Apple Developer ID (the current state)

With no secrets set, the app is ad hoc signed. Users who download it have to
confirm the first launch (System Settings › Privacy & Security › Open Anyway,
or `xattr -dr com.apple.quarantine /Applications/BenCode.app`). The release
notes, the README and the landing page already carry these instructions.

An ad hoc signature differs from one build to the next, so after each update
macOS may ask again for permissions already granted (Documents, Desktop…).

## Turning on signing and notarization

This requires the Apple Developer Program (99 USD/year). It has nothing to do
with the App Store: a Developer ID only makes Gatekeeper trust a build
downloaded from the web.

1. Create a **Developer ID Application** certificate (Xcode › Settings ›
   Accounts › Manage Certificates, or developer.apple.com › Certificates), then
   export it from Keychain Access as a password-protected `.p12` file.
2. Create an app-specific password at [account.apple.com](https://account.apple.com)
   › Sign-In and Security › App-Specific Passwords.
3. Add these secrets under repo › Settings › Secrets and variables › Actions:

   | Secret | Value |
   | :--- | :--- |
   | `MACOS_CERTIFICATE` | `base64 -i DeveloperID.p12 \| pbcopy` |
   | `MACOS_CERTIFICATE_PASSWORD` | the `.p12` file's password |
   | `APPLE_ID` | the developer account's Apple ID email |
   | `APPLE_TEAM_ID` | the 10-character Team ID (developer.apple.com › Membership) |
   | `APPLE_APP_PASSWORD` | the app-specific password from step 2 |

4. From the next release on, the workflow signs the app with the hardened
   runtime and `packaging/macos/entitlements.plist`, notarizes and staples the
   dmg, and drops the "First launch" section from the release notes.
5. Remove the "Confirm the first launch" step from the landing page
   (`#first-launch` in `site/index.html`) and step 3 of Install in the README.

The first time signing is on, try a signed build (Actions › Release › Run
workflow): open the app, run an agent turn, open the terminal, and try a
command that needs a permission (for example `ls ~/Documents`). If the hardened
runtime blocks something, add the matching entitlement to `entitlements.plist`.
