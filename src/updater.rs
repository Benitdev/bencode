//! MonoCode `tauri-plugin-updater` (`app/model/updater.ts`, the `updater`
//! block of `tauri.conf.json`): the release feed, the signed archive, and
//! putting the new BenCode.app in place of the running one.
//!
//! A release publishes `latest.json` (Tauri's shape: `version`, `notes`,
//! `pub_date` and a `darwin-<arch>` entry with the archive's `url` and its
//! minisign `signature`, plus its `size`) beside each architecture's
//! `BenCode-<arch>.app.tar.gz`. The archive is checked against the release
//! key built in at `BENCODE_UPDATE_PUBKEY`; a build without one does not
//! update itself.
//!
//! Everything here blocks; the app runs it on the background executor.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;

/// The newest release's feed; GitHub redirects it to the asset.
pub const FEED_URL: &str =
    "https://github.com/Benitdev/bencode/releases/latest/download/latest.json";
/// Where a build that cannot update itself sends people.
pub const RELEASES_URL: &str = "https://github.com/Benitdev/bencode/releases/latest";
/// The release key's public half (minisign: its base64 line, or the whole
/// `.pub` file), given to the release build by CI.
const PUBLIC_KEY: Option<&str> = option_env!("BENCODE_UPDATE_PUBKEY");
/// The installed-update marker in the data folder (MonoCode
/// `monocode.installedUpdate`).
const INSTALLED_MARKER: &str = "installed-update";

pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Why this copy of BenCode does not update itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NotConfigured {
    /// Built without the release key (`cargo run`, a local bundle).
    NoKey,
    /// Not running from a `.app` bundle.
    NoBundle,
}

/// A newer release, ready to download.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Update {
    pub version: String,
    pub notes: Option<String>,
    url: String,
    signature: String,
    size: Option<u64>,
}

/// What the feed says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Check {
    Current,
    Available(Update),
}

/// How far a download has come, shared with the app while it runs.
#[derive(Default)]
pub struct Progress {
    downloaded: AtomicU64,
    total: AtomicU64,
}

impl Progress {
    /// Whole percent, once the size is known.
    pub fn percent(&self) -> Option<u8> {
        let total = self.total.load(Ordering::Relaxed);
        (total > 0).then(|| {
            let done = self.downloaded.load(Ordering::Relaxed).min(total);
            (done * 100 / total) as u8
        })
    }
}

#[derive(Deserialize)]
struct Feed {
    version: String,
    #[serde(default)]
    notes: Option<String>,
    #[serde(default)]
    platforms: std::collections::BTreeMap<String, FeedPlatform>,
}

#[derive(Deserialize)]
struct FeedPlatform {
    url: String,
    signature: String,
    #[serde(default)]
    size: Option<u64>,
}

/// The release key and the running bundle, or why there is none.
pub fn configured() -> Result<PathBuf, NotConfigured> {
    if PUBLIC_KEY.is_none_or(|key| key.trim().is_empty()) {
        return Err(NotConfigured::NoKey);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| bundle_of(&exe))
        .ok_or(NotConfigured::NoBundle)
}

/// Fetches the feed and says whether it names a newer version for this Mac.
pub fn check() -> Result<Check> {
    let response = crate::rate_limits::http::send(
        FEED_URL,
        &[("Accept", "application/json")],
        crate::rate_limits::http::Send {
            follow_redirects: true,
            ..Default::default()
        },
        Duration::from_secs(30),
    )?;
    match response.status {
        200 => {}
        // No release, or one published without a feed.
        404 => return Ok(Check::Current),
        status => bail!("the release feed answered HTTP {status}"),
    }
    parse_feed(&response.body, current_version(), &platform_key())
}

/// `darwin-aarch64` or `darwin-x86_64`, as Tauri names them.
fn platform_key() -> String {
    format!("darwin-{}", std::env::consts::ARCH)
}

fn parse_feed(body: &str, current: &str, platform: &str) -> Result<Check> {
    let feed: Feed = serde_json::from_str(body).context("the release feed is not valid")?;
    let version = feed.version.trim().trim_start_matches('v').to_string();
    if !is_newer(&version, current) {
        return Ok(Check::Current);
    }
    let entry = feed
        .platforms
        .get(platform)
        .with_context(|| format!("the release has no build for {platform}"))?;
    // It goes on curl's command line: nothing that could read as an option.
    if !entry.url.starts_with("https://") {
        bail!("the release feed's download link is not https");
    }
    Ok(Check::Available(Update {
        version,
        notes: feed
            .notes
            .map(|notes| notes.trim().to_string())
            .filter(|notes| !notes.is_empty()),
        url: entry.url.clone(),
        signature: entry.signature.clone(),
        size: entry.size,
    }))
}

/// `candidate` is a later version than `current` (`major.minor.patch`,
/// a pre-release sorting before its release).
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// `(major, minor, patch, is_release, pre-release)`: a release sorts after
/// any pre-release of the same numbers.
fn parse_version(text: &str) -> Option<(u64, u64, u64, bool, String)> {
    let text = text.trim().trim_start_matches('v');
    let (numbers, pre) = match text.split_once('-') {
        Some((numbers, pre)) => (numbers, pre.to_string()),
        None => (text, String::new()),
    };
    let mut parts = numbers.split('.').map(|part| part.parse::<u64>().ok());
    let (major, minor, patch) = (parts.next()??, parts.next()??, parts.next()??);
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, pre.is_empty(), pre))
}

/// `BenCode.app` for an executable at `BenCode.app/Contents/MacOS/bencode`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let bundle = exe.parent()?.parent()?.parent()?;
    let looks_right = exe.parent()?.file_name()? == "MacOS"
        && exe.parent()?.parent()?.file_name()? == "Contents"
        && bundle.extension()? == "app";
    looks_right.then(|| bundle.to_path_buf())
}

/// The archive matches the release key's signature.
fn verify_signature(bytes: &[u8], signature: &str, public_key: &str) -> Result<()> {
    use minisign_verify::{PublicKey, Signature};
    let public_key = public_key.trim();
    let key = if public_key.contains('\n') {
        PublicKey::decode(public_key)
    } else {
        PublicKey::from_base64(public_key)
    }
    .map_err(|err| anyhow!("the release key built into BenCode is not valid: {err}"))?;
    let signature = Signature::decode(signature.trim())
        .map_err(|err| anyhow!("the update's signature is not valid: {err}"))?;
    key.verify(bytes, &signature, false)
        .map_err(|_| anyhow!("the download does not match BenCode's release signature"))
}

/// Downloads, checks and puts the update in place of `bundle`; BenCode then
/// needs a restart ([`relaunch`]). The old bundle is removed.
pub fn install(update: &Update, bundle: &Path, progress: &Progress) -> Result<()> {
    let public_key = PUBLIC_KEY.context("this build has no release key")?;
    if bundle
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation")
    {
        bail!(
            "macOS is running BenCode from a temporary copy. Move BenCode to your \
             Applications folder, open it from there, and update again."
        );
    }
    let parent = bundle.parent().context("BenCode's folder is unknown")?;
    // Beside the bundle, so the swap is a rename on one volume.
    let staging = parent.join(format!(".BenCode-update-{}", std::process::id()));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)
            .with_context(|| format!("could not clear {}", staging.display()))?;
    }
    std::fs::create_dir(&staging).with_context(|| {
        format!(
            "BenCode cannot write to {}. Move it to your Applications folder, or \
             download the update from {RELEASES_URL}",
            parent.display()
        )
    })?;
    let result = install_into(update, bundle, &staging, public_key, progress);
    if let Err(err) = std::fs::remove_dir_all(&staging) {
        log::warn!("could not remove {}: {err}", staging.display());
    }
    result
}

fn install_into(
    update: &Update,
    bundle: &Path,
    staging: &Path,
    public_key: &str,
    progress: &Progress,
) -> Result<()> {
    let archive = staging.join("BenCode.app.tar.gz");
    progress
        .total
        .store(update.size.unwrap_or(0), Ordering::Relaxed);
    download(&update.url, &archive, progress)?;
    let bytes = std::fs::read(&archive).context("could not read the download")?;
    verify_signature(&bytes, &update.signature, public_key)?;
    drop(bytes);

    run("tar", |cmd| {
        cmd.arg("-xzf").arg(&archive).arg("-C").arg(staging)
    })
    .context("could not unpack the update")?;
    let fresh = staging.join("BenCode.app");
    let (want_id, got_id) = (
        bundle_value(bundle, "CFBundleIdentifier")?,
        bundle_value(&fresh, "CFBundleIdentifier")?,
    );
    if want_id != got_id {
        bail!("the update is {got_id}, not {want_id}");
    }
    let got_version = bundle_value(&fresh, "CFBundleShortVersionString")?;
    if got_version != update.version {
        bail!("the update says {got_version}, not {}", update.version);
    }
    run("codesign", |cmd| {
        cmd.args(["--verify", "--strict"]).arg(&fresh)
    })
    .context("the update's code signature does not check out")?;

    // Old aside, new in, old gone; the running process keeps its files.
    let old = staging.join("BenCode.app.old");
    std::fs::rename(bundle, &old).context("could not move the current BenCode aside")?;
    if let Err(err) = std::fs::rename(&fresh, bundle) {
        if let Err(back) = std::fs::rename(&old, bundle) {
            log::error!("could not put the current BenCode back: {back}");
        }
        return Err(err).context("could not put the update in place");
    }
    Ok(())
}

/// curl to `dest`, with the file's growing size as the progress.
fn download(url: &str, dest: &Path, progress: &Progress) -> Result<()> {
    let mut child = Command::new("curl")
        .args([
            "-q",
            "--fail",
            "--location",
            // A redirect must not leave https.
            "--proto",
            "=https",
            "--proto-redir",
            "=https",
            "--silent",
            "--show-error",
            "--max-time",
            "1800",
        ])
        .arg("--output")
        .arg(dest)
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("could not start curl")?;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Ok(meta) = std::fs::metadata(dest) {
            progress.downloaded.store(meta.len(), Ordering::Relaxed);
        }
        std::thread::sleep(Duration::from_millis(150));
    };
    if !status.success() {
        let mut detail = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            std::io::Read::read_to_string(&mut stderr, &mut detail).ok();
        }
        bail!(
            "download failed: {}",
            detail.trim().trim_start_matches("curl: ")
        );
    }
    if let Ok(meta) = std::fs::metadata(dest) {
        progress.downloaded.store(meta.len(), Ordering::Relaxed);
        progress.total.store(
            meta.len().max(progress.total.load(Ordering::Relaxed)),
            Ordering::Relaxed,
        );
    }
    Ok(())
}

/// One key of a bundle's `Info.plist`.
fn bundle_value(bundle: &Path, key: &str) -> Result<String> {
    let plist = bundle.join("Contents/Info.plist");
    let output = Command::new("plutil")
        .args(["-extract", key, "raw", "-o", "-"])
        .arg(&plist)
        .output()
        .context("could not run plutil")?;
    if !output.status.success() {
        bail!("{} has no {key}", plist.display());
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn run(program: &str, args: impl FnOnce(&mut Command) -> &mut Command) -> Result<()> {
    let mut command = Command::new(program);
    args(&mut command);
    let output = command
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("could not run {program}"))?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

/// Opens `bundle` once this process has exited (MonoCode `relaunch`); the
/// caller quits right after.
pub fn relaunch(bundle: &Path) -> Result<()> {
    use std::os::unix::process::CommandExt as _;
    Command::new("/bin/sh")
        .arg("-c")
        .arg(r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done; exec /usr/bin/open "$2""#)
        .arg("bencode-relaunch")
        .arg(std::process::id().to_string())
        .arg(bundle)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        // Its own group, so it outlives BenCode's.
        .process_group(0)
        .spawn()
        .context("could not schedule the restart")?;
    Ok(())
}

/// MonoCode `rememberInstalledUpdate`: the next launch says "Updated to".
pub fn remember_installed(version: &str) {
    let Some(dir) = crate::storage::data_dir() else {
        return;
    };
    if let Err(err) = std::fs::write(dir.join(INSTALLED_MARKER), version) {
        log::warn!("could not note the installed update: {err}");
    }
}

/// MonoCode `consumeInstalledUpdate`: the version just installed, once.
pub fn take_installed() -> Option<String> {
    let path = crate::storage::data_dir()?.join(INSTALLED_MARKER);
    let version = std::fs::read_to_string(&path).ok()?;
    if let Err(err) = std::fs::remove_file(&path) {
        log::warn!("could not clear the installed-update note: {err}");
    }
    Some(version.trim().to_string()).filter(|version| !version.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/updater");

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(format!("{FIXTURES}/{name}")).expect(name)
    }

    fn feed(version: &str) -> String {
        format!(
            r#"{{"version":"{version}","notes":" BenCode {version} ","pub_date":"2026-10-10T00:00:00Z",
                "platforms":{{"darwin-aarch64":{{"url":"https://example.com/a.tar.gz","signature":"sig","size":42}}}}}}"#
        )
    }

    #[test]
    fn versions_compare_by_number_then_pre_release() {
        assert!(is_newer("0.1.1", "0.1.0"));
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.9.0"));
        assert!(is_newer("0.2.0", "0.2.0-beta.1"));
        assert!(!is_newer("0.2.0-beta.1", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.0.9", "0.1.0"));
        assert!(!is_newer("nonsense", "0.1.0"));
        assert!(!is_newer("1.2", "0.1.0"));
    }

    #[test]
    fn a_newer_feed_names_this_macs_build() {
        let check = parse_feed(&feed("0.2.0"), "0.1.0", "darwin-aarch64").unwrap();
        let Check::Available(update) = check else {
            panic!("expected an update: {check:?}");
        };
        assert_eq!(update.version, "0.2.0");
        assert_eq!(update.notes.as_deref(), Some("BenCode 0.2.0"));
        assert_eq!(update.url, "https://example.com/a.tar.gz");
        assert_eq!(update.size, Some(42));
    }

    #[test]
    fn the_same_version_is_current() {
        assert_eq!(
            parse_feed(&feed("0.1.0"), "0.1.0", "darwin-aarch64").unwrap(),
            Check::Current
        );
    }

    #[test]
    fn a_feed_without_this_mac_is_an_error() {
        let err = parse_feed(&feed("0.2.0"), "0.1.0", "darwin-x86_64").unwrap_err();
        assert!(err.to_string().contains("darwin-x86_64"), "{err}");
    }

    #[test]
    fn a_download_link_must_be_https() {
        let feed = feed("0.2.0").replace("https://example.com/a.tar.gz", "--config=/etc/x");
        let err = parse_feed(&feed, "0.1.0", "darwin-aarch64").unwrap_err();
        assert!(err.to_string().contains("https"), "{err}");
    }

    #[test]
    fn the_bundle_is_three_levels_up() {
        let exe = Path::new("/Applications/BenCode.app/Contents/MacOS/bencode");
        assert_eq!(
            bundle_of(exe),
            Some(PathBuf::from("/Applications/BenCode.app"))
        );
        assert_eq!(bundle_of(Path::new("/repo/target/debug/bencode")), None);
    }

    #[test]
    fn a_signed_archive_verifies_with_its_key() {
        let bytes = std::fs::read(format!("{FIXTURES}/payload.bin")).unwrap();
        let signature = fixture("payload.bin.minisig");
        let key_file = fixture("test.pub");
        let key_line = key_file.lines().nth(1).unwrap();
        verify_signature(&bytes, &signature, key_line).expect("base64 key");
        verify_signature(&bytes, &signature, &key_file).expect("whole .pub file");
    }

    #[test]
    fn a_changed_archive_does_not_verify() {
        let mut bytes = std::fs::read(format!("{FIXTURES}/payload.bin")).unwrap();
        bytes.push(b'!');
        let key_file = fixture("test.pub");
        let err = verify_signature(&bytes, &fixture("payload.bin.minisig"), &key_file).unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    #[test]
    fn progress_is_a_percentage_once_the_size_is_known() {
        let progress = Progress::default();
        assert_eq!(progress.percent(), None);
        progress.total.store(200, Ordering::Relaxed);
        progress.downloaded.store(50, Ordering::Relaxed);
        assert_eq!(progress.percent(), Some(25));
        progress.downloaded.store(500, Ordering::Relaxed);
        assert_eq!(progress.percent(), Some(100));
    }
}
