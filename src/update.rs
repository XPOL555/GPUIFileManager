//! Update check against the project's GitHub Releases. Only reports what it found;
//! the About dialog offers to open the release page.

use gpui_kit::*;
use serde::Deserialize;

use crate::shell;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPO_URL: &str = env!("CARGO_PKG_REPOSITORY");

const API_HOST: &str = "api.github.com";
/// The latest release's JSON is a few KB; anything bigger is cut off.
const MAX_RESPONSE: usize = 1 << 20;

/// Result of the last check, shared by every window.
#[derive(Clone, Debug, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available { version: String, url: String },
    Failed,
}

impl Global for UpdateStatus {}

pub fn status(cx: &App) -> UpdateStatus {
    cx.try_global::<UpdateStatus>().cloned().unwrap_or_default()
}

/// Starts a check in the background, unless one is already running.
pub fn check(cx: &mut App) {
    if matches!(status(cx), UpdateStatus::Checking) {
        return;
    }
    cx.set_global(UpdateStatus::Checking);
    cx.refresh_windows();
    let task = cx.background_spawn(async { fetch_latest() });
    cx.spawn(async move |cx| {
        let status = task.await.unwrap_or_else(|err| {
            eprintln!("update check: {err}");
            UpdateStatus::Failed
        });
        cx.update(|cx| {
            cx.set_global(status);
            cx.refresh_windows();
        });
    })
    .detach();
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
}

/// Blocking: asks GitHub for the latest release and compares it with this build.
fn fetch_latest() -> Result<UpdateStatus, String> {
    let repo = REPO_URL.trim_end_matches('/').strip_prefix("https://github.com/").ok_or("repository is not on GitHub")?;
    let path = format!("/repos/{repo}/releases/latest");
    let headers = "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n";
    let (status, body) = shell::https_get(API_HOST, &path, headers, MAX_RESPONSE)?;
    match status {
        // No release published yet.
        404 => Ok(UpdateStatus::UpToDate),
        200 => {
            let release: Release = serde_json::from_slice(&body).map_err(|e| format!("bad response: {e}"))?;
            Ok(if is_newer(&release.tag_name, VERSION) {
                let version = release.tag_name.trim_start_matches(['v', 'V']).to_string();
                UpdateStatus::Available { version, url: release.html_url }
            } else {
                UpdateStatus::UpToDate
            })
        }
        other => Err(format!("HTTP {other}")),
    }
}

/// "v1.2.3", "1.2" or "1.2.3-beta" → (1, 2, 3). Pre-release suffixes are ignored.
fn parse_version(s: &str) -> Option<(u64, u64, u64)> {
    let core = s.trim().trim_start_matches(['v', 'V']);
    let core = core.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|p| p.parse::<u64>());
    let major = parts.next()?.ok()?;
    let minor = parts.next().unwrap_or(Ok(0)).ok()?;
    let patch = parts.next().unwrap_or(Ok(0)).ok()?;
    Some((major, minor, patch))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_newer, parse_version};

    #[test]
    fn parses_versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.4"), Some((0, 4, 0)));
        assert_eq!(parse_version("2.0.1-beta.1"), Some((2, 0, 1)));
        assert_eq!(parse_version("nightly"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("v0.2.0", "0.1.0"));
        assert!(is_newer("v0.1.10", "0.1.9"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("v0.0.9", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }
}
