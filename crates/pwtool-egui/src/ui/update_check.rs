//! Check for updates (`TUpdateCheckThread`, `upstream/src/util/
//! UpdateCheck.cpp`). The port asks the GitHub Releases API of its own
//! repository (`PORT_REPOSITORY`) for the latest release instead of the
//! version file of the original web site.

use std::time::Duration;

use egui::ViewportId;
use pwtool_app::program::{PORT_REPOSITORY, PORT_VERSION, PROGRAM_NAME};
use pwtool_core::language::{trl, trl_format};

use super::dialogs::ask;
use crate::vcl::dialogs::{MsgButtons, MsgIcon, MsgResult};

/// `TUpdateCheckThread::CheckResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckResult {
    Negative,
    Positive,
    Error,
    Timeout,
    /// No repository configured (port only).
    NotConfigured,
}

/// Latest release.
struct Release {
    version: String,
    url: String,
    note: String,
}

/// "owner/repo" from `PORT_REPOSITORY` ("https://github.com/owner/repo" or
/// "owner/repo").
fn repo_path() -> Option<String> {
    // test hook: another repository
    let env = std::env::var("PWTOOL_UPDATE_REPO").ok();
    let r = env.as_deref().unwrap_or(PORT_REPOSITORY).trim().trim_end_matches('/').trim_end_matches(".git");
    let r = r.strip_prefix("https://github.com/").unwrap_or(r);
    let parts: Vec<&str> = r.split('/').filter(|x| !x.is_empty()).collect();
    (parts.len() == 2).then(|| format!("{}/{}", parts[0], parts[1]))
}

/// HTTPS GET in the calling thread: status and body, or (error, timed out).
fn https_get(url: &str, timeout_secs: u64) -> Result<(u16, Vec<u8>), (String, bool)> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout_secs)))
        .http_status_as_error(false)
        .build()
        .into();
    let map_err = |e: ureq::Error| {
        let timed_out = matches!(e, ureq::Error::Timeout(_));
        (e.to_string(), timed_out)
    };
    let mut resp = agent
        .get(url)
        .header("User-Agent", "pwtool-update-check")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(map_err)?;
    let status = resp.status().as_u16();
    let body = resp.body_mut().with_config().limit(4 << 20).read_to_vec().map_err(map_err)?;
    Ok((status, body))
}

fn latest_release(repo: &str, timeout_secs: u64) -> Result<Release, (String, bool)> {
    let (status, body) = https_get(&format!("https://api.github.com/repos/{repo}/releases/latest"), timeout_secs)?;
    if !(200..=299).contains(&status) {
        return Err((format!("{} ({status})", trl("Could not download version file")), false));
    }
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| (e.to_string(), false))?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).unwrap_or("");
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let url = v.get("html_url").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let note = v.get("name").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<u32>().is_err()) || url.is_empty() {
        return Err((trl("Version or URL not specified in version file, or unknown file format"), false));
    }
    Ok(Release { version, url, note })
}

/// `CheckForUpdates()`: with `show_error`, errors are reported here.
pub async fn check_for_updates(parent: ViewportId, show_error: bool) -> CheckResult {
    let Some(repo) = repo_path() else { return CheckResult::NotConfigured };
    // automatic start-up check: short timeout; manual check: 15 s
    let timeout_secs = if show_error { 15 } else { 3 };
    let (tx, rx) = async_channel::bounded(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(latest_release(&repo, timeout_secs));
    });
    let r = rx.recv().await.unwrap_or_else(|_| Err(("internal error".into(), false)));
    match r {
        Ok(r) => {
            if pwtool_app::app::compare_versions(&r.version, PORT_VERSION) > 0 {
                let mut m = trl_format("A new version (%1) of %2 is available!\nDo you want to visit the download page now?", &[&r.version, PROGRAM_NAME]);
                if !r.note.is_empty() && r.note != r.version && r.note.trim_start_matches(['v', 'V']) != r.version {
                    m.push_str("\n\n");
                    m.push_str(&trl_format("(NOTE: %1)", &[&r.note]));
                }
                if ask(parent, &m, MsgIcon::Info, MsgButtons::YesNo, false).await == MsgResult::Yes {
                    pwtool_app::platform::sys::system_open(std::ffi::OsStr::new(&r.url));
                }
                CheckResult::Positive
            } else {
                CheckResult::Negative
            }
        }
        Err((e, timed_out)) => {
            if show_error {
                ask(parent, &trl_format("Error while checking for updates:\n%1.", &[&e]), MsgIcon::Error, MsgButtons::Ok, false).await;
            }
            if timed_out { CheckResult::Timeout } else { CheckResult::Error }
        }
    }
}
