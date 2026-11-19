//! Check for updates (`TUpdateCheckThread`, `upstream/src/util/
//! UpdateCheck.cpp`). The port asks the GitHub Releases API of its own
//! repository (`PORT_REPOSITORY`) for the latest release instead of the
//! version file of the original web site.

use gtk4 as gtk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use pwtech_core::language::{trl, trl_format};

use super::util::{MsgButtons, MsgIcon, MsgResult, msg_box, open_uri};
use crate::program::{PORT_REPOSITORY, PORT_VERSION, PROGRAM_NAME};

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

/// Latest release: (version, page URL, note).
struct Release {
    version: String,
    url: String,
    note: String,
}

/// "owner/repo" from `PORT_REPOSITORY` ("https://github.com/owner/repo" or
/// "owner/repo").
fn repo_path() -> Option<String> {
    // test hook: another repository
    let env = std::env::var("PWTECH_UPDATE_REPO").ok();
    let r = env.as_deref().unwrap_or(PORT_REPOSITORY).trim().trim_end_matches('/').trim_end_matches(".git");
    let r = r.strip_prefix("https://github.com/").unwrap_or(r);
    let parts: Vec<&str> = r.split('/').filter(|x| !x.is_empty()).collect();
    (parts.len() == 2).then(|| format!("{}/{}", parts[0], parts[1]))
}

/// Decode a body with `Transfer-Encoding: chunked`.
fn dechunk(mut body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(nl) = body.windows(2).position(|w| w == b"\r\n") else { break };
        let size_str = String::from_utf8_lossy(&body[..nl]);
        let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16).unwrap_or(0);
        body = &body[nl + 2..];
        if size == 0 || body.len() < size {
            break;
        }
        out.extend_from_slice(&body[..size]);
        body = &body[(size + 2).min(body.len())..];
    }
    out
}

/// HTTPS GET with GIO (TLS from glib-networking). Returns status and body.
async fn https_get(host: &str, path: &str) -> Result<(u32, Vec<u8>), String> {
    let client = gio::SocketClient::new();
    client.set_tls(true);
    client.set_timeout(15);
    let conn = client.connect_to_host_future(host, 443).await.map_err(|e| e.to_string())?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nUser-Agent: pwtech-update-check\r\nAccept: application/vnd.github+json\r\nConnection: close\r\n\r\n"
    );
    conn.output_stream()
        .write_all_future(req.into_bytes(), glib::Priority::DEFAULT)
        .await
        .map_err(|(_, e)| e.to_string())?;
    let input = conn.input_stream();
    let mut data = Vec::new();
    loop {
        let buf = vec![0u8; 16384];
        let (buf, n) = input.read_future(buf, glib::Priority::DEFAULT).await.map_err(|(_, e)| e.to_string())?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() > 4 << 20 {
            return Err("response too large".into());
        }
    }
    let _ = conn.close_future(glib::Priority::DEFAULT).await;
    let head_end = data.windows(4).position(|w| w == b"\r\n\r\n").ok_or("invalid HTTP response")?;
    let head = String::from_utf8_lossy(&data[..head_end]).to_string();
    let status: u32 = head.split_whitespace().nth(1).and_then(|s| s.parse().ok()).ok_or("invalid HTTP response")?;
    let mut body = data[head_end + 4..].to_vec();
    if head.lines().any(|l| l.to_ascii_lowercase().replace(' ', "") == "transfer-encoding:chunked") {
        body = dechunk(&body);
    }
    Ok((status, body))
}

async fn latest_release(repo: &str) -> Result<Release, String> {
    let (status, body) = https_get("api.github.com", &format!("/repos/{repo}/releases/latest")).await?;
    if !(200..=299).contains(&status) {
        return Err(format!("{} ({status})", trl("Could not download version file")));
    }
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let tag = v.get("tag_name").and_then(|t| t.as_str()).unwrap_or("");
    let version = tag.trim_start_matches(['v', 'V']).to_string();
    let url = v.get("html_url").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let note = v.get("name").and_then(|t| t.as_str()).unwrap_or("").to_string();
    let parts: Vec<&str> = version.split('.').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.parse::<u32>().is_err()) || url.is_empty() {
        return Err(trl("Version or URL not specified in version file, or unknown file format"));
    }
    Ok(Release { version, url, note })
}

/// `CheckForUpdates()`: with `show_error`, errors (and "up to date") are
/// reported by the caller/here.
pub async fn check_for_updates(parent: Option<&gtk::Window>, show_error: bool) -> CheckResult {
    let Some(repo) = repo_path() else { return CheckResult::NotConfigured };
    match latest_release(&repo).await {
        Ok(r) => {
            if crate::app::compare_versions(&r.version, PORT_VERSION) > 0 {
                let mut m = trl_format(
                    "A new version (%1) of %2 is available!\nDo you want to visit the download page now?",
                    &[&r.version, PROGRAM_NAME],
                );
                if !r.note.is_empty() && r.note != r.version && r.note.trim_start_matches(['v', 'V']) != r.version {
                    m.push_str("\n\n");
                    m.push_str(&trl_format("(NOTE: %1)", &[&r.note]));
                }
                if msg_box(parent, &m, MsgIcon::Info, MsgButtons::YesNo, false).await == MsgResult::Yes {
                    open_uri(parent, &r.url);
                }
                CheckResult::Positive
            } else {
                CheckResult::Negative
            }
        }
        Err(e) => {
            if show_error {
                msg_box(
                    parent,
                    &trl_format("Error while checking for updates:\n%1.", &[&e]),
                    MsgIcon::Error,
                    MsgButtons::Ok,
                    false,
                )
                .await;
            }
            if e.contains("timed out") { CheckResult::Timeout } else { CheckResult::Error }
        }
    }
}
