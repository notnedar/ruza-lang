use crate::crypto_lz4::sha256_file;
use crate::patch_engine::LogLevel;
use serde::{Deserialize, Serialize};
use std::ffi::c_void;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::ptr;
use std::thread;
use std::time::Duration;
use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders,
    WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest, WinHttpSetOption,
    WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_REDIRECT_POLICY,
    WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP, WINHTTP_QUERY_CONTENT_LENGTH,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const LATEST_RELEASE_API: &str = "https://api.github.com/repos/notnedar/ruza-lang/releases/latest";
const ALLOWED_HOSTS: &[&str] = &[
    "api.github.com",
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub release_url: String,
    pub exe_url: String,
    pub sha_url: String,
    pub exe_size: u64,
}

struct WinHttpHandle(*mut c_void);

impl Drop for WinHttpHandle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn parse_semver(s: &str) -> Option<(u32, u32, u32)> {
    let trimmed = s.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u32>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u32>().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(latest: &str, current: &str) -> bool {
    match (parse_semver(latest), parse_semver(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

pub fn parse_sha256sums(text: &str, filename: &str) -> Option<String> {
    let target = filename.to_ascii_lowercase();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let hash = parts.next()?;
        let name = parts.next().unwrap_or("").trim_start_matches('*');
        if hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_hexdigit())
            && (name.is_empty() || name.to_ascii_lowercase() == target)
        {
            return Some(hash.to_ascii_uppercase());
        }
    }
    None
}

pub fn validate_allowed_https_url(url: &str) -> Result<(String, String), String> {
    let rest = url
        .strip_prefix("https://")
        .ok_or_else(|| format!("Rejected non-HTTPS URL: {}", url))?;
    let (host_port, path) = match rest.find('/') {
        Some(idx) => (&rest[..idx], &rest[idx..]),
        None => (rest, "/"),
    };
    let host = host_port.split(':').next().unwrap_or("").to_ascii_lowercase();
    if !ALLOWED_HOSTS.iter().any(|allowed| *allowed == host) {
        return Err(format!("Rejected untrusted update host: {}", host));
    }
    Ok((host, path.to_string()))
}

fn winhttp_stream<W, P>(
    url: &str,
    extra_headers: &str,
    mut on_chunk: W,
    mut on_progress: P,
) -> Result<u64, String>
where
    W: FnMut(&[u8]) -> Result<(), String>,
    P: FnMut(u64, u64),
{
    let (host, path) = validate_allowed_https_url(url)?;
    let ua = to_wide(&format!("Ruza-Lang/{}", CURRENT_VERSION));
    let host_w = to_wide(&host);
    let verb_w = to_wide("GET");
    let path_w = to_wide(&path);
    let headers_w = if extra_headers.is_empty() {
        None
    } else {
        Some(to_wide(extra_headers))
    };

    unsafe {
        let mut raw_session = WinHttpOpen(
            ua.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        );
        if raw_session.is_null() {
            raw_session = WinHttpOpen(
                ua.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                ptr::null(),
                ptr::null(),
                0,
            );
        }
        if raw_session.is_null() {
            return Err("WinHttpOpen failed".into());
        }
        let session = WinHttpHandle(raw_session);
        let _ = WinHttpSetTimeouts(session.0, 10_000, 10_000, 15_000, 30_000);

        let raw_conn = WinHttpConnect(session.0, host_w.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if raw_conn.is_null() {
            return Err(format!("WinHttpConnect failed for {}", host));
        }
        let conn = WinHttpHandle(raw_conn);

        let raw_req = WinHttpOpenRequest(
            conn.0,
            verb_w.as_ptr(),
            path_w.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if raw_req.is_null() {
            return Err("WinHttpOpenRequest failed".into());
        }
        let req = WinHttpHandle(raw_req);

        let redirect_policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_DISALLOW_HTTPS_TO_HTTP;
        let _ = WinHttpSetOption(
            req.0,
            WINHTTP_OPTION_REDIRECT_POLICY,
            &redirect_policy as *const u32 as *const c_void,
            std::mem::size_of::<u32>() as u32,
        );

        let (hdr_ptr, hdr_len) = match &headers_w {
            Some(w) => (w.as_ptr(), (w.len() - 1) as u32),
            None => (ptr::null(), 0),
        };
        if WinHttpSendRequest(req.0, hdr_ptr, hdr_len, ptr::null(), 0, 0, 0) == 0 {
            return Err(format!("HTTPS request failed for {} (check internet connection)", host));
        }
        if WinHttpReceiveResponse(req.0, ptr::null_mut()) == 0 {
            return Err(format!("No HTTPS response received from {}", host));
        }

        let mut status_code: u32 = 0;
        let mut status_len = std::mem::size_of::<u32>() as u32;
        if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            &mut status_code as *mut u32 as *mut c_void,
            &mut status_len,
            ptr::null_mut(),
        ) == 0
        {
            return Err("Failed reading HTTP status code".into());
        }
        if status_code != 200 {
            return Err(format!("GitHub returned HTTP {}", status_code));
        }

        let mut content_len: u32 = 0;
        let mut cl_size = std::mem::size_of::<u32>() as u32;
        let total_bytes = if WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER,
            ptr::null(),
            &mut content_len as *mut u32 as *mut c_void,
            &mut cl_size,
            ptr::null_mut(),
        ) != 0
        {
            content_len as u64
        } else {
            0
        };

        let mut buf = vec![0u8; 64 * 1024];
        let mut downloaded = 0u64;
        loop {
            let mut bytes_read: u32 = 0;
            if WinHttpReadData(
                req.0,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as u32,
                &mut bytes_read,
            ) == 0
            {
                return Err("Interrupted while downloading response body".into());
            }
            if bytes_read == 0 {
                break;
            }
            on_chunk(&buf[..bytes_read as usize])?;
            downloaded += bytes_read as u64;
            on_progress(downloaded, total_bytes);
        }
        Ok(downloaded)
    }
}

fn winhttp_get_string(url: &str, extra_headers: &str) -> Result<String, String> {
    let mut bytes = Vec::new();
    winhttp_stream(
        url,
        extra_headers,
        |chunk| {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                return Err("Metadata response exceeded 2 MB limit".into());
            }
            bytes.extend_from_slice(chunk);
            Ok(())
        },
        |_, _| {},
    )?;
    String::from_utf8(bytes).map_err(|e| format!("Invalid UTF-8 response: {}", e))
}

pub fn check_for_update() -> Result<UpdateInfo, String> {
    let body = winhttp_get_string(
        LATEST_RELEASE_API,
        "Accept: application/vnd.github+json\r\nX-GitHub-Api-Version: 2022-11-28\r\n",
    )?;
    let v: serde_json::Value =
        serde_json::from_str(&body).map_err(|e| format!("Parse GitHub release JSON failed: {}", e))?;

    let latest_version = v
        .get("tag_name")
        .and_then(|x| x.as_str())
        .ok_or("Missing tag_name in GitHub release")?
        .trim()
        .to_string();
    let release_url = v
        .get("html_url")
        .and_then(|x| x.as_str())
        .unwrap_or("https://github.com/notnedar/ruza-lang/releases/latest")
        .to_string();

    let mut exe_url = String::new();
    let mut sha_url = String::new();
    let mut exe_size = 0u64;
    if let Some(assets) = v.get("assets").and_then(|x| x.as_array()) {
        for a in assets {
            let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("");
            let dl = a
                .get("browser_download_url")
                .and_then(|x| x.as_str())
                .unwrap_or("");
            if name.eq_ignore_ascii_case("RuzaLangPatcher.exe") {
                exe_url = dl.to_string();
                exe_size = a.get("size").and_then(|x| x.as_u64()).unwrap_or(0);
            } else if name.eq_ignore_ascii_case("SHA256SUMS.txt") {
                sha_url = dl.to_string();
            }
        }
    }

    if !exe_url.is_empty() {
        let _ = validate_allowed_https_url(&exe_url)?;
    }
    if !sha_url.is_empty() {
        let _ = validate_allowed_https_url(&sha_url)?;
    }

    let has_update = is_newer_version(&latest_version, CURRENT_VERSION);
    Ok(UpdateInfo {
        current_version: format!("v{}", CURRENT_VERSION),
        latest_version,
        has_update,
        release_url,
        exe_url,
        sha_url,
        exe_size,
    })
}

fn companion_path(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

pub fn download_and_apply_update<L, P>(
    info: &UpdateInfo,
    mut log: L,
    mut on_progress: P,
) -> Result<PathBuf, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(u64, u64, &str),
{
    if info.exe_url.is_empty() || info.sha_url.is_empty() {
        return Err("Release assets (RuzaLangPatcher.exe + SHA256SUMS.txt) are missing.".into());
    }
    let _ = validate_allowed_https_url(&info.exe_url)?;
    let _ = validate_allowed_https_url(&info.sha_url)?;

    log(
        &format!(
            "=== Starting Auto-Update: {} -> {} ===",
            info.current_version, info.latest_version
        ),
        LogLevel::Info,
    );
    on_progress(0, info.exe_size.max(1), "Fetching official SHA256SUMS.txt...");

    let sha_text = winhttp_get_string(&info.sha_url, "")?;
    let expected_sha = parse_sha256sums(&sha_text, "RuzaLangPatcher.exe")
        .ok_or("Could not parse RuzaLangPatcher.exe checksum from SHA256SUMS.txt")?;
    log(
        &format!("  Expected SHA-256: {}...", &expected_sha[..16]),
        LogLevel::Info,
    );

    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Failed resolving current executable path: {}", e))?;
    let tmp_exe = companion_path(&current_exe, ".update.tmp");
    let old_exe = companion_path(&current_exe, ".old");
    let _ = fs::remove_file(&tmp_exe);

    let dl_res = (|| -> Result<u64, String> {
        let mut f = File::create(&tmp_exe)
            .map_err(|e| format!("Failed creating temp update file: {}", e))?;
        let mut last_pct: u64 = 999;
        let expected_total = info.exe_size;
        let written = winhttp_stream(
            &info.exe_url,
            "",
            |chunk| f.write_all(chunk).map_err(|e| format!("Write update failed: {}", e)),
            |done, hdr_tot| {
                let tot = if hdr_tot > 0 { hdr_tot } else { expected_total.max(1) };
                let pct = ((done * 100) / tot).min(100);
                if pct != last_pct && (pct % 5 == 0 || pct == 100) {
                    last_pct = pct;
                    let mb_done = done as f64 / (1024.0 * 1024.0);
                    let mb_tot = tot as f64 / (1024.0 * 1024.0);
                    on_progress(
                        done,
                        tot,
                        &format!(
                            "Downloading {} ({:.1} / {:.1} MB)...",
                            info.latest_version, mb_done, mb_tot
                        ),
                    );
                }
            },
        )?;
        f.sync_all().map_err(|e| format!("Flush update failed: {}", e))?;
        Ok(written)
    })();

    let written = match dl_res {
        Ok(n) => n,
        Err(e) => {
            let _ = fs::remove_file(&tmp_exe);
            return Err(e);
        }
    };

    on_progress(written, written.max(1), "Verifying SHA-256 checksum...");
    let actual_sha = sha256_file(&tmp_exe)?;
    if !actual_sha.eq_ignore_ascii_case(&expected_sha) {
        let _ = fs::remove_file(&tmp_exe);
        return Err(format!(
            "SHA-256 verification failed! Expected {} but got {}",
            expected_sha, actual_sha
        ));
    }
    log(
        &format!("  SHA-256 verified 100% ({} bytes).", written),
        LogLevel::Ok,
    );

    // Atomic Windows in-place swap: NTFS allows renaming a running executable
    let _ = fs::remove_file(&old_exe);
    fs::rename(&current_exe, &old_exe)
        .map_err(|e| format!("Failed staging previous executable: {}", e))?;
    if let Err(e) = fs::rename(&tmp_exe, &current_exe) {
        let _ = fs::rename(&old_exe, &current_exe);
        let _ = fs::remove_file(&tmp_exe);
        return Err(format!("Failed activating new executable: {}", e));
    }

    log(
        &format!("  Updated executable ready! Launching {}...", info.latest_version),
        LogLevel::Ok,
    );
    if let Err(e) = Command::new(&current_exe)
        .arg("--cleanup-old")
        .arg(&old_exe)
        .spawn()
    {
        return Err(format!(
            "Updated successfully, but auto-relaunch failed (please re-open manually): {}",
            e
        ));
    }
    Ok(current_exe)
}

pub fn schedule_old_exe_cleanup(args: &[String]) {
    let mut targets: Vec<PathBuf> = Vec::new();
    if args.len() >= 3 && args[1].eq_ignore_ascii_case("--cleanup-old") {
        targets.push(PathBuf::from(&args[2]));
    }
    if let Ok(cur) = std::env::current_exe() {
        let default_old = companion_path(&cur, ".old");
        if default_old.is_file() && !targets.contains(&default_old) {
            targets.push(default_old);
        }
        let stale_tmp = companion_path(&cur, ".update.tmp");
        if stale_tmp.is_file() {
            let _ = fs::remove_file(&stale_tmp);
        }
    }
    if targets.is_empty() {
        return;
    }
    thread::spawn(move || {
        for _ in 0..25 {
            targets.retain(|p| p.is_file() && fs::remove_file(p).is_err());
            if targets.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(160));
        }
    });
}

pub fn open_safe_release_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://github.com/notnedar/ruza-lang") {
        return Err("Refused to open non-repository URL".into());
    }
    Command::new("explorer")
        .arg(url)
        .spawn()
        .map_err(|e| format!("Open browser failed: {}", e))?;
    Ok(())
}
