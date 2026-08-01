//! In-place GitHub Releases auto-updater for the installed Apksule folder.
//!
//! Downloads the portable `apksule-windows-x64.exe` (never the Inno Setup),
//! verifies SHA-256, replaces `apksule.exe` (and optional `apksule.ico`) in the
//! directory of the running binary, then relaunches.

use std::env;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

const OWNER: &str = "Overl1te";
const REPO: &str = "Apksule";
const PORTABLE_ASSET: &str = "apksule-windows-x64.exe";
const ICON_ASSET: &str = "apksule.ico";
const CHECKSUMS_ASSET: &str = "SHA256SUMS.txt";
const USER_AGENT: &str = concat!("Apksule/", env!("APKSULE_VERSION"));
const HTTP_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("HTTP request failed: {0}")]
    Http(String),
    #[error("failed to parse GitHub release JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid semver '{0}': {1}")]
    Semver(String, String),
    #[error("release asset '{0}' was not found")]
    MissingAsset(String),
    #[error("SHA-256 mismatch for {asset}: expected {expected}, got {actual}")]
    ChecksumMismatch { asset: String, expected: String, actual: String },
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("in-place replace failed: {0}")]
    Replace(String),
    #[error("failed to relaunch Apksule: {0}")]
    Relaunch(String),
    #[error("current executable path is invalid")]
    InvalidExePath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub current: Version,
    pub latest: Version,
    pub tag: String,
    pub asset_name: String,
    pub download_url: String,
    pub icon_url: Option<String>,
    pub checksum_url: Option<String>,
    pub html_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    UpToDate { current: Version },
    Updated { from: Version, to: Version },
}

#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    html_url: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

#[must_use]
pub fn current_version() -> Version {
    Version::parse(env!("APKSULE_VERSION")).unwrap_or_else(|_| Version::new(0, 0, 0))
}

/// Look up the newest non-draft, non-prerelease GitHub release newer than this build.
pub fn check_for_update() -> Result<Option<AvailableUpdate>, UpdateError> {
    let current = current_version();
    let release = fetch_latest_release()?;
    let latest = parse_tag_version(&release.tag_name)?;
    if latest <= current {
        return Ok(None);
    }

    let asset = release
        .assets
        .iter()
        .find(|asset| asset.name == PORTABLE_ASSET)
        .ok_or_else(|| UpdateError::MissingAsset(PORTABLE_ASSET.to_owned()))?;
    let icon_url = release
        .assets
        .iter()
        .find(|asset| asset.name == ICON_ASSET)
        .map(|asset| asset.browser_download_url.clone());
    let checksum_url = release
        .assets
        .iter()
        .find(|asset| asset.name == CHECKSUMS_ASSET)
        .map(|asset| asset.browser_download_url.clone());

    Ok(Some(AvailableUpdate {
        current,
        latest,
        tag: release.tag_name,
        asset_name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        icon_url,
        checksum_url,
        html_url: release.html_url,
    }))
}

/// Download the portable build and replace files in the installed application folder.
///
/// Never downloads or runs the Inno Setup installer. On success this process exits
/// after scheduling / performing relaunch.
pub fn apply_update(
    update: &AvailableUpdate,
    relaunch_args: &[String],
) -> Result<UpdateOutcome, UpdateError> {
    let exe_path = env::current_exe()?;
    let install_dir = exe_path.parent().ok_or(UpdateError::InvalidExePath)?.to_path_buf();
    let pending_dir = update_staging_dir(&update.tag);
    fs::create_dir_all(&pending_dir)?;

    let staged_exe = pending_dir.join("apksule.exe");
    let staged_ico = pending_dir.join("apksule.ico");

    tracing::info!(
        from = %update.current,
        to = %update.latest,
        install_dir = %install_dir.display(),
        asset = %update.asset_name,
        "downloading in-place Apksule update (portable exe, not Inno Setup)"
    );
    download_file(&update.download_url, &staged_exe)?;

    if let Some(checksum_url) = &update.checksum_url {
        let sums = download_text(checksum_url)?;
        let expected = expected_sha256(&sums, &update.asset_name).ok_or_else(|| {
            UpdateError::MissingAsset(format!("checksum for {}", update.asset_name))
        })?;
        let actual = sha256_file(&staged_exe)?;
        if !actual.eq_ignore_ascii_case(&expected) {
            return Err(UpdateError::ChecksumMismatch {
                asset: update.asset_name.clone(),
                expected,
                actual,
            });
        }
        tracing::info!(sha256 = %actual, "update checksum verified");
    } else {
        tracing::warn!("SHA256SUMS.txt missing from release; installing without checksum");
    }

    if let Some(icon_url) = &update.icon_url {
        match download_file(icon_url, &staged_ico) {
            Ok(()) => tracing::info!("staged apksule.ico for install folder"),
            Err(error) => tracing::warn!(%error, "optional icon download failed"),
        }
    }

    let writable = directory_is_writable(&install_dir);
    if writable {
        replace_in_place(&staged_exe, &staged_ico, &install_dir)?;
        tracing::info!(
            from = %update.current,
            to = %update.latest,
            dir = %install_dir.display(),
            "files replaced in install folder; relaunching"
        );
        relaunch(&exe_path, relaunch_args)?;
    } else {
        tracing::info!(
            dir = %install_dir.display(),
            "install folder is not writable; scheduling deferred in-place replace"
        );
        schedule_deferred_replace(
            &staged_exe,
            &staged_ico,
            &install_dir,
            &exe_path,
            relaunch_args,
        )?;
        // Current process must exit so the deferred copy can overwrite apksule.exe.
        std::process::exit(0);
    }

    Ok(UpdateOutcome::Updated { from: update.current.clone(), to: update.latest.clone() })
}

/// Check and, when newer, install + relaunch. Failures are returned to the caller.
pub fn check_and_update(relaunch_args: &[String]) -> Result<UpdateOutcome, UpdateError> {
    match check_for_update()? {
        None => Ok(UpdateOutcome::UpToDate { current: current_version() }),
        Some(update) => {
            tracing::info!(
                current = %update.current,
                latest = %update.latest,
                url = %update.html_url,
                "update available"
            );
            apply_update(&update, relaunch_args)
        }
    }
}

fn replace_in_place(
    staged_exe: &Path,
    staged_ico: &Path,
    install_dir: &Path,
) -> Result<(), UpdateError> {
    // Windows cannot overwrite a running image; self_replace renames aside then swaps.
    self_replace::self_replace(staged_exe)
        .map_err(|error| UpdateError::Replace(error.to_string()))?;

    if staged_ico.is_file() {
        let target_ico = install_dir.join("apksule.ico");
        if let Err(error) = fs::copy(staged_ico, &target_ico) {
            tracing::warn!(%error, path = %target_ico.display(), "could not refresh apksule.ico");
        }
    }
    Ok(())
}

fn schedule_deferred_replace(
    staged_exe: &Path,
    staged_ico: &Path,
    install_dir: &Path,
    target_exe: &Path,
    relaunch_args: &[String],
) -> Result<(), UpdateError> {
    let pending_dir = staged_exe.parent().ok_or(UpdateError::InvalidExePath)?;
    let script_path = pending_dir.join("apply-in-place.ps1");
    let launcher_path = pending_dir.join("apply-in-place.cmd");

    let args_literal = powershell_arg_list(relaunch_args);
    let icon_block = if staged_ico.is_file() {
        format!(
            r"
$iconSrc = '{icon}'
$iconDst = Join-Path $TargetDir 'apksule.ico'
Copy-Item -LiteralPath $iconSrc -Destination $iconDst -Force
",
            icon = powershell_literal(staged_ico)
        )
    } else {
        String::new()
    };

    let script = format!(
        r"
$ErrorActionPreference = 'Stop'
$ParentPid = {pid}
$SourceExe = '{source_exe}'
$TargetDir = '{target_dir}'
$TargetExe = '{target_exe}'
{icon_block}
try {{
  Wait-Process -Id $ParentPid -Timeout 120 -ErrorAction SilentlyContinue
}} catch {{}}
Start-Sleep -Milliseconds 700
$destExe = Join-Path $TargetDir 'apksule.exe'
Copy-Item -LiteralPath $SourceExe -Destination $destExe -Force
$env:APKSULE_SKIP_UPDATE = '1'
$argList = @({args})
if ($argList.Count -gt 0) {{
  Start-Process -FilePath $TargetExe -ArgumentList $argList -WorkingDirectory $TargetDir
}} else {{
  Start-Process -FilePath $TargetExe -WorkingDirectory $TargetDir
}}
",
        pid = std::process::id(),
        source_exe = powershell_literal(staged_exe),
        target_dir = powershell_literal(install_dir),
        target_exe = powershell_literal(target_exe),
        icon_block = icon_block,
        args = args_literal,
    );
    fs::write(&script_path, script)?;

    let launcher = format!(
        "@echo off\r\npowershell -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \"{}\"\r\n",
        script_path.display()
    );
    fs::write(&launcher_path, launcher)?;

    // Elevate when needed so Program Files installs can be overwritten without Inno.
    let elevated = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "Start-Process -FilePath '{}' -Verb RunAs -WindowStyle Hidden",
                powershell_literal(&launcher_path)
            ),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    match elevated {
        Ok(status) if status.success() => Ok(()),
        Ok(_) | Err(_) => {
            // Fallback without UAC: works for per-user / writable install folders.
            Command::new("cmd")
                .args(["/C"])
                .arg(&launcher_path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|error| UpdateError::Replace(error.to_string()))?;
            Ok(())
        }
    }
}

fn update_staging_dir(tag: &str) -> PathBuf {
    let base = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .or_else(|| env::var_os("TEMP").map(PathBuf::from))
        .unwrap_or_else(env::temp_dir);
    base.join("Apksule").join("updates").join(tag)
}

fn directory_is_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".apksule-write-test-{}", std::process::id()));
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = fs::remove_file(probe);
            true
        }
        Err(_) => false,
    }
}

fn powershell_literal(path: &Path) -> String {
    path.display().to_string().replace('\'', "''")
}

fn powershell_arg_list(args: &[String]) -> String {
    args.iter().map(|arg| format!("'{}'", arg.replace('\'', "''"))).collect::<Vec<_>>().join(",")
}

fn fetch_latest_release() -> Result<GhRelease, UpdateError> {
    let url = format!("https://api.github.com/repos/{OWNER}/{REPO}/releases/latest");
    let body = http_get_bytes(&url)?;
    let release: GhRelease = serde_json::from_slice(&body)?;
    if release.draft || release.prerelease {
        return Err(UpdateError::Http("latest release is draft/prerelease".to_owned()));
    }
    if release.assets.iter().all(|asset| asset.size == 0) {
        tracing::warn!("latest release has empty assets metadata");
    }
    Ok(release)
}

fn parse_tag_version(tag: &str) -> Result<Version, UpdateError> {
    let trimmed = tag.trim().trim_start_matches('v');
    Version::parse(trimmed).map_err(|error| UpdateError::Semver(tag.to_owned(), error.to_string()))
}

fn http_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(HTTP_TIMEOUT))
        .user_agent(USER_AGENT)
        .build()
        .into()
}

fn http_get_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    let response = http_agent()
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .call()
        .map_err(|error| UpdateError::Http(error.to_string()))?;
    if !(200..300).contains(&response.status().as_u16()) {
        return Err(UpdateError::Http(format!(
            "unexpected status {} for {url}",
            response.status()
        )));
    }
    let mut body = Vec::new();
    response
        .into_body()
        .as_reader()
        .take(64 * 1024 * 1024)
        .read_to_end(&mut body)
        .map_err(|error| UpdateError::Http(error.to_string()))?;
    Ok(body)
}

fn download_text(url: &str) -> Result<String, UpdateError> {
    let bytes = http_get_bytes(url)?;
    String::from_utf8(bytes).map_err(|error| UpdateError::Http(error.to_string()))
}

fn download_file(url: &str, path: &Path) -> Result<(), UpdateError> {
    let bytes = http_get_bytes(url)?;
    let mut file = fs::File::create(path)?;
    file.write_all(&bytes)?;
    file.flush()?;
    Ok(())
}

fn expected_sha256(sums: &str, asset_name: &str) -> Option<String> {
    for line in sums.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        if name == asset_name {
            return Some(hash.to_owned());
        }
    }
    None
}

fn sha256_file(path: &Path) -> Result<String, UpdateError> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 16 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    Ok(hex)
}

fn relaunch(exe: &Path, args: &[String]) -> Result<(), UpdateError> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .env("APKSULE_SKIP_UPDATE", "1")
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command.spawn().map_err(|error| UpdateError::Relaunch(error.to_string()))?;
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_checksum_lines() {
        let sums = "\
abc123  apksule-windows-x64.exe
def456 *Apksule-Setup-0.1.1-windows-x64.exe
";
        assert_eq!(expected_sha256(sums, "apksule-windows-x64.exe").as_deref(), Some("abc123"));
        assert_eq!(
            expected_sha256(sums, "Apksule-Setup-0.1.1-windows-x64.exe").as_deref(),
            Some("def456")
        );
    }

    #[test]
    fn parses_v_prefix_tags() {
        assert_eq!(parse_tag_version("v0.1.1").unwrap(), Version::new(0, 1, 1));
        assert_eq!(parse_tag_version("0.2.0").unwrap(), Version::new(0, 2, 0));
    }

    #[test]
    fn powershell_escaping_doubles_quotes() {
        assert_eq!(
            powershell_literal(Path::new(r"C:\Program Files\Apksule")),
            r"C:\Program Files\Apksule"
        );
        assert_eq!(powershell_literal(Path::new(r"C:\O'Brien\Apksule")), r"C:\O''Brien\Apksule");
    }
}
