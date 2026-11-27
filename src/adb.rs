use crate::crypto_lz4::sha256_file;
use crate::patch_engine::{
    self, ClientVersionInfo, LogLevel, PatchLang, TargetStatus, ANDROID_EXPECTED_RES_REV,
    ANDROID_EXPECTED_VERSION, ANDROID_PACKAGE_ID, ANDROID_STATIC_UI_EN_SHA,
    ANDROID_STATIC_UI_ID_SHA, ANDROID_STATIC_UI_ORIG_SHA, ANDROID_STATIC_UI_SUB_OFFSET,
    ANDROID_TOTAL_STRINGS, ANDROID_VSTRING_EN_SHA, ANDROID_VSTRING_ID_SHA,
    ANDROID_VSTRING_ORIG_SHA,
};
use serde::Serialize;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub const REMOTE_FILES_ROOT: &str = "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files";
pub const REMOTE_VSTRING: &str =
    "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/raw_base_package/BundleFiles/1qnamndresrohzjhuvt2lw.r";
pub const REMOTE_STATIC_UI: &str =
    "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/default_package/BundleFiles/nibdvaxcnfh3en5bnvhtw.r";
pub const REMOTE_RAW_MANIFEST_DIR: &str =
    "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/raw_base_package/ManifestFiles";
pub const REMOTE_DEFAULT_MANIFEST_DIR: &str =
    "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/default_package/ManifestFiles";

#[derive(Clone, Serialize)]
pub struct AndroidDevice {
    pub serial: String,
    pub state: String, // "device" | "unauthorized" | "offline"
    pub model: String,
    pub android_ver: String,
    pub is_emulator: bool,
    pub has_game: bool,
}

#[derive(Clone, Serialize)]
pub struct AndroidScanResult {
    pub adb_path: String,
    pub serial: String,
    pub installed: bool,
    pub running_pids: Vec<u32>,
    pub version: ClientVersionInfo,
    pub targets: Vec<TargetStatus>,
    pub backup_dir: String,
    pub news_cache_bytes: u64,
    pub has_news_backup: bool,
}

/// Starts the ADB server with null stdio handles so the forked background daemon
/// never inherits stdout/stderr pipe handles on Windows (which would otherwise block `output()`).
pub fn ensure_adb_server(adb: &Path) -> Result<(), String> {
    let status = Command::new(adb)
        .arg("start-server")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Failed starting ADB server ({}): {}", adb.display(), e))?;
    if !status.success() {
        return Err(format!("ADB start-server exited with {}", status));
    }
    Ok(())
}

fn run_adb_raw(adb: &Path, args: &[&str]) -> Result<(bool, String, String), String> {
    let out = Command::new(adb)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("Failed launching ADB ({}): {}", adb.display(), e))?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    Ok((out.status.success(), stdout, stderr))
}

fn filter_daemon_lines(text: &str) -> String {
    text.lines()
        .filter(|l| {
            let t = l.trim_start();
            !t.starts_with("* ") && !t.starts_with("adb server version")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn adb_shell(adb: &Path, serial: &str, cmd: &str) -> Result<String, String> {
    let (ok, stdout, stderr) = run_adb_raw(adb, &["-s", serial, "shell", cmd])?;
    let clean = filter_daemon_lines(&stdout);
    if !ok && clean.trim().is_empty() {
        return Err(format!(
            "ADB shell failed on {}: {}",
            serial,
            filter_daemon_lines(&stderr).trim()
        ));
    }
    Ok(clean)
}

fn adb_pull(adb: &Path, serial: &str, remote: &str, local: &Path) -> Result<(), String> {
    if let Some(parent) = local.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create local dir failed: {}", e))?;
    }
    let local_s = local.to_string_lossy().to_string();
    let (ok, stdout, stderr) = run_adb_raw(adb, &["-s", serial, "pull", remote, &local_s])?;
    if !ok || !local.is_file() {
        let detail = format!(
            "{} {}",
            filter_daemon_lines(&stdout).trim(),
            filter_daemon_lines(&stderr).trim()
        );
        return Err(format!("ADB pull failed for {}: {}", remote, detail.trim()));
    }
    Ok(())
}

fn adb_push(adb: &Path, serial: &str, local: &Path, remote: &str) -> Result<(), String> {
    let local_s = local.to_string_lossy().to_string();
    let (ok, stdout, stderr) = run_adb_raw(adb, &["-s", serial, "push", &local_s, remote])?;
    if !ok {
        let detail = format!(
            "{} {}",
            filter_daemon_lines(&stdout).trim(),
            filter_daemon_lines(&stderr).trim()
        );
        return Err(format!("ADB push failed for {}: {}", remote, detail.trim()));
    }
    Ok(())
}

pub fn detect_adb_binary(custom_path: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = custom_path {
        let pb = PathBuf::from(p.trim());
        if pb.is_file() {
            return Some(pb);
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    // 1. Next to RuzaLangPatcher.exe (portable platform-tools drop-in)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("adb.exe"));
            candidates.push(dir.join("platform-tools").join("adb.exe"));
        }
    }
    // 2. Android SDK Platform-Tools & PATH
    if let Some(la) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(&la).join(r"Android\Sdk\platform-tools\adb.exe"));
    }
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            candidates.push(dir.join("adb.exe"));
        }
    }
    candidates.push(PathBuf::from(r"C:\platform-tools\adb.exe"));
    // 3. Popular Android emulators on Windows (BlueStacks, LDPlayer, MuMu, MEmu, Nox)
    for drive in ['C', 'D', 'E', 'F'] {
        candidates.push(PathBuf::from(format!(r"{}:\Program Files\BlueStacks_nxt\HD-Adb.exe", drive)));
        candidates.push(PathBuf::from(format!(r"{}:\Program Files\BlueStacks\HD-Adb.exe", drive)));
        candidates.push(PathBuf::from(format!(r"{}:\LDPlayer\LDPlayer9\adb.exe", drive)));
        candidates.push(PathBuf::from(format!(r"{}:\LDPlayer\LDPlayer4.0\adb.exe", drive)));
        candidates.push(PathBuf::from(format!(
            r"{}:\Program Files\Netease\MuMuPlayerGlobal-12.0\shell\adb.exe",
            drive
        )));
        candidates.push(PathBuf::from(format!(
            r"{}:\Program Files\Netease\MuMu Player 12\shell\adb.exe",
            drive
        )));
        candidates.push(PathBuf::from(format!(r"{}:\Program Files\Microvirt\MEmu\adb.exe", drive)));
        candidates.push(PathBuf::from(format!(r"{}:\Program Files (x86)\Nox\bin\nox_adb.exe", drive)));
    }

    for c in candidates {
        if c.is_file() {
            return Some(c);
        }
    }
    None
}

fn parse_devices_output(stdout: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || trimmed.starts_with("List of devices")
            || trimmed.starts_with('*')
        {
            continue;
        }
        let mut parts = trimmed.split_whitespace();
        let Some(serial) = parts.next() else { continue };
        let Some(state) = parts.next() else { continue };
        let mut model = String::new();
        for tok in parts {
            if let Some(m) = tok.strip_prefix("model:") {
                model = m.replace('_', " ");
            }
        }
        out.push((serial.to_string(), state.to_string(), model));
    }
    out
}

pub fn connect_tcp_endpoint(adb: &Path, endpoint: &str) -> Result<String, String> {
    let trimmed = endpoint.trim();
    if trimmed.is_empty()
        || trimmed.len() > 80
        || !trimmed
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b':' | b'-'))
        || !trimmed.contains(':')
    {
        return Err(
            "Invalid ADB endpoint. Use IP:Port format (example: 192.168.1.25:41235 or 127.0.0.1:5555)."
                .into(),
        );
    }
    ensure_adb_server(adb)?;
    let (ok, stdout, stderr) = run_adb_raw(adb, &["connect", trimmed])?;
    let msg = format!(
        "{} {}",
        filter_daemon_lines(&stdout).trim(),
        filter_daemon_lines(&stderr).trim()
    )
    .trim()
    .to_string();
    let lower = msg.to_ascii_lowercase();
    if !ok || lower.contains("failed") || lower.contains("cannot connect") || lower.contains("unable") {
        return Err(if msg.is_empty() {
            format!("Could not connect to {}", trimmed)
        } else {
            msg
        });
    }
    Ok(if msg.is_empty() {
        format!("Connected to {}", trimmed)
    } else {
        msg
    })
}

pub fn list_devices(adb: &Path) -> Result<Vec<AndroidDevice>, String> {
    ensure_adb_server(adb)?;
    let (_, stdout, _) = run_adb_raw(adb, &["devices", "-l"])?;
    let mut raw = parse_devices_output(&stdout);

    // If no devices yet, gently try BlueStacks default local ADB port (127.0.0.1:5555)
    if raw.is_empty() {
        let _ = run_adb_raw(adb, &["connect", "127.0.0.1:5555"]);
        let (_, stdout2, _) = run_adb_raw(adb, &["devices", "-l"])?;
        raw = parse_devices_output(&stdout2);
    }

    let mut devices = Vec::with_capacity(raw.len());
    for (serial, state, mut model) in raw {
        let is_emulator = serial.starts_with("emulator-")
            || serial.starts_with("127.0.0.1:")
            || serial.starts_with("localhost:");
        let mut android_ver = String::new();
        let mut has_game = false;
        if state == "device" {
            let probe = format!(
                "getprop ro.product.model; echo __SEP__; getprop ro.build.version.release; echo __SEP__; pm path {} 2>/dev/null || true",
                ANDROID_PACKAGE_ID
            );
            if let Ok(props) = adb_shell(adb, &serial, &probe) {
                let mut sp = props.split("__SEP__");
                let m = sp.next().unwrap_or("").trim();
                let v = sp.next().unwrap_or("").trim();
                let pkg = sp.next().unwrap_or("");
                if !m.is_empty() {
                    model = m.to_string();
                }
                if !v.is_empty() {
                    android_ver = v.to_string();
                }
                has_game = pkg.contains("package:");
            }
        }
        if model.is_empty() {
            model = if is_emulator {
                "Android Emulator".to_string()
            } else {
                "Android Device".to_string()
            };
        }
        devices.push(AndroidDevice {
            serial,
            state,
            model,
            android_ver,
            is_emulator,
            has_game,
        });
    }
    // Put ready devices with the target package installed first
    devices.sort_by_key(|d| (!(d.state == "device" && d.has_game), d.state != "device"));
    Ok(devices)
}

pub fn find_running_android_pids(adb: &Path, serial: &str) -> Vec<u32> {
    let cmd = format!("pidof {} 2>/dev/null || true", ANDROID_PACKAGE_ID);
    let Ok(out) = adb_shell(adb, serial, &cmd) else {
        return Vec::new();
    };
    out.split_whitespace()
        .filter_map(|tok| tok.parse::<u32>().ok())
        .collect()
}

pub fn ensure_android_not_running(adb: &Path, serial: &str, action: &str) -> Result<(), String> {
    let pids = find_running_android_pids(adb, serial);
    if !pids.is_empty() {
        let s = pids
            .iter()
            .map(|p| format!("PID {}", p))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "Blocked: Android client is currently running ({}). Close the app on Android before {}!",
            s, action
        ));
    }
    Ok(())
}

pub fn sanitize_serial(serial: &str) -> String {
    let s: String = serial
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "default".to_string()
    } else {
        s
    }
}

pub fn android_backup_dir(serial: &str) -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("RuzaLang")
        .join("AndroidBackups")
        .join(sanitize_serial(serial))
}

fn sha_matches(sha: &str, expected: &str) -> bool {
    !expected.is_empty() && sha.eq_ignore_ascii_case(expected)
}

fn remote_sha256(adb: &Path, serial: &str, remote_path: &str) -> Result<Option<String>, String> {
    let cmd = format!("if [ -f '{}' ]; then sha256sum '{}'; else echo __MISSING__; fi", remote_path, remote_path);
    let out = adb_shell(adb, serial, &cmd)?;
    for line in out.lines() {
        let t = line.trim();
        if t == "__MISSING__" {
            return Ok(None);
        }
        if let Some(tok) = t.split_whitespace().next() {
            if tok.len() == 64 && tok.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(Some(tok.to_ascii_uppercase()));
            }
        }
    }
    Ok(None)
}

fn parse_best_manifest_rev(listing: &str) -> String {
    let mut best_num: Option<u64> = None;
    let mut best_str = String::new();
    for tok in listing.split_whitespace() {
        let Some(stem) = tok.strip_suffix(".bytes") else {
            continue;
        };
        if !stem.contains("_package_") {
            continue;
        }
        if let Some(rev) = stem.rsplit('_').next() {
            if !rev.is_empty() && rev.bytes().all(|b| b.is_ascii_digit()) {
                if let Ok(num) = rev.parse::<u64>() {
                    if best_num.map(|b| num > b).unwrap_or(true) {
                        best_num = Some(num);
                        best_str = rev.to_string();
                    }
                }
            }
        }
    }
    best_str
}

pub fn scan_android_device<F>(
    adb: &Path,
    serial: &str,
    mut log: F,
) -> Result<AndroidScanResult, String>
where
    F: FnMut(&str, LogLevel),
{
    ensure_adb_server(adb)?;
    let bak_dir = android_backup_dir(serial);
    let pkg_check = adb_shell(
        adb,
        serial,
        &format!("pm path {} 2>/dev/null || true", ANDROID_PACKAGE_ID),
    )?;
    let installed = pkg_check.contains("package:");
    let running_pids = if installed {
        find_running_android_pids(adb, serial)
    } else {
        Vec::new()
    };

    if !installed {
        log(
            &format!("  [ANDROID] Target package not installed on {}.", serial),
            LogLevel::Warn,
        );
        return Ok(AndroidScanResult {
            adb_path: adb.to_string_lossy().to_string(),
            serial: serial.to_string(),
            installed: false,
            running_pids,
            version: ClientVersionInfo {
                detected_version: String::new(),
                expected_version: ANDROID_EXPECTED_VERSION.into(),
                detected_res_rev: String::new(),
                expected_res_rev: ANDROID_EXPECTED_RES_REV.into(),
                detected_build_guid: String::new(),
                expected_build_guid: String::new(),
                status: "mismatch".into(),
                reasons: vec!["Android client is not installed on this device".into()],
            },
            targets: Vec::new(),
            backup_dir: bak_dir.to_string_lossy().to_string(),
            news_cache_bytes: 0,
            has_news_backup: false,
        });
    }

    // Read versionName, manifest revision, and news Library size in one fast shell command
    let meta_cmd = format!(
        "dumpsys package {} | grep 'versionName=' | head -n 1; echo __M1__; ls '{}' '{}' 2>/dev/null || true; echo __M2__; stat -c %s '{}/Library' 2>/dev/null || echo 0",
        ANDROID_PACKAGE_ID, REMOTE_RAW_MANIFEST_DIR, REMOTE_DEFAULT_MANIFEST_DIR, REMOTE_FILES_ROOT
    );
    let meta_out = adb_shell(adb, serial, &meta_cmd)?;
    let mut parts = meta_out.split("__M1__");
    let ver_part = parts.next().unwrap_or("");
    let rest1 = parts.next().unwrap_or("");
    let mut parts2 = rest1.split("__M2__");
    let man_part = parts2.next().unwrap_or("");
    let news_sz_str = parts2.next().unwrap_or("0").trim();

    let detected_version = ver_part
        .split("versionName=")
        .nth(1)
        .map(|s| s.split_whitespace().next().unwrap_or("").trim().to_string())
        .unwrap_or_default();
    let detected_res_rev = parse_best_manifest_rev(man_part);
    let news_cache_bytes = news_sz_str.parse::<u64>().unwrap_or(0);
    let has_news_backup = bak_dir
        .join("NewsWork")
        .join("_NewsPatchBackup")
        .join("Library")
        .is_file();

    let specs = [
        (
            "vstring",
            "VStringPool",
            "Story, Quests, Skills, Kibo, Settings",
            REMOTE_VSTRING,
            "1qnamndresrohzjhuvt2lw.r.original",
            74055usize,
            ANDROID_VSTRING_ORIG_SHA,
            ANDROID_VSTRING_EN_SHA,
            ANDROID_VSTRING_ID_SHA,
        ),
        (
            "static_ui",
            "Static UI KeyConfig",
            "Prefab Buttons, Headers, Notices",
            REMOTE_STATIC_UI,
            "nibdvaxcnfh3en5bnvhtw.r.original",
            3135usize,
            ANDROID_STATIC_UI_ORIG_SHA,
            ANDROID_STATIC_UI_EN_SHA,
            ANDROID_STATIC_UI_ID_SHA,
        ),
    ];

    let mut targets = Vec::with_capacity(2);
    for (id, name, sub, remote, bak_name, count, orig_sha, en_sha, id_sha) in specs {
        let bak = bak_dir.join(bak_name);
        let has_backup = bak.is_file();
        let sha_opt = remote_sha256(adb, serial, remote)?;
        let Some(sha) = sha_opt else {
            log(&format!("  [MISSING] Android {}", name), LogLevel::Warn);
            targets.push(TargetStatus {
                id: id.into(),
                name: name.into(),
                subtitle: sub.into(),
                rel_path: remote.into(),
                backup_name: bak_name.into(),
                string_count: count,
                state: "missing".into(),
                has_backup,
                sha256: String::new(),
            });
            continue;
        };

        let state = if sha_matches(&sha, id_sha) {
            "patched_id"
        } else if sha_matches(&sha, en_sha) {
            "patched_en"
        } else if sha_matches(&sha, orig_sha) {
            "original"
        } else {
            "modified"
        };
        let state_lbl = match state {
            "patched_en" => "ENGLISH",
            "patched_id" => "INDONESIAN",
            "original" => "ORIGINAL JP",
            "modified" => "MODIFIED",
            _ => "MISSING",
        };
        let lvl = if state == "patched_en" || state == "patched_id" {
            LogLevel::Ok
        } else {
            LogLevel::Info
        };
        log(
            &format!(
                "  [ANDROID {}] {} ({} strings, backup={})",
                state_lbl,
                name,
                count,
                if has_backup { "YES" } else { "NO" }
            ),
            lvl,
        );
        targets.push(TargetStatus {
            id: id.into(),
            name: name.into(),
            subtitle: sub.into(),
            rel_path: remote.into(),
            backup_name: bak_name.into(),
            string_count: count,
            state: state.into(),
            has_backup,
            sha256: sha,
        });
    }

    let mut reasons = Vec::new();
    if !detected_version.is_empty() && detected_version != ANDROID_EXPECTED_VERSION {
        reasons.push(format!(
            "Android version {} != {}",
            detected_version, ANDROID_EXPECTED_VERSION
        ));
    }
    if !detected_res_rev.is_empty() && detected_res_rev != ANDROID_EXPECTED_RES_REV {
        reasons.push(format!(
            "Android resource revision r{} != r{}",
            detected_res_rev, ANDROID_EXPECTED_RES_REV
        ));
    }
    let modified_cnt = targets.iter().filter(|t| t.state == "modified").count();
    let missing_cnt = targets.iter().filter(|t| t.state == "missing").count();
    if modified_cnt > 0 {
        reasons.push(format!(
            "{} Android bundle(s) differ from verified SHA-256 hashes",
            modified_cnt
        ));
    }
    if missing_cnt > 0 {
        reasons.push(format!(
            "{} Android bundle(s) missing (finish in-game data download first)",
            missing_cnt
        ));
    }

    let status = if reasons.is_empty() {
        "synced".to_string()
    } else {
        "mismatch".to_string()
    };

    Ok(AndroidScanResult {
        adb_path: adb.to_string_lossy().to_string(),
        serial: serial.to_string(),
        installed: true,
        running_pids,
        version: ClientVersionInfo {
            detected_version,
            expected_version: ANDROID_EXPECTED_VERSION.into(),
            detected_res_rev,
            expected_res_rev: ANDROID_EXPECTED_RES_REV.into(),
            detected_build_guid: String::new(),
            expected_build_guid: String::new(),
            status,
            reasons,
        },
        targets,
        backup_dir: bak_dir.to_string_lossy().to_string(),
        news_cache_bytes,
        has_news_backup,
    })
}

fn push_atomic_verified(
    adb: &Path,
    serial: &str,
    local_file: &Path,
    remote_target: &str,
    expected_sha: &str,
) -> Result<String, String> {
    let remote_tmp = format!("{}.ruza_tmp", remote_target);
    adb_push(adb, serial, local_file, &remote_tmp)?;
    let tmp_sha = remote_sha256(adb, serial, &remote_tmp)?
        .ok_or_else(|| format!("Staged remote file missing after push: {}", remote_tmp))?;
    if !tmp_sha.eq_ignore_ascii_case(expected_sha) {
        let _ = adb_shell(adb, serial, &format!("rm -f '{}'", remote_tmp));
        return Err(format!(
            "Remote transfer SHA-256 mismatch ({} != {})",
            tmp_sha, expected_sha
        ));
    }
    let mv_cmd = format!("mv -f '{}' '{}' && echo MV_OK", remote_tmp, remote_target);
    let mv_out = adb_shell(adb, serial, &mv_cmd)?;
    if !mv_out.contains("MV_OK") {
        let _ = adb_shell(adb, serial, &format!("rm -f '{}'", remote_tmp));
        return Err(format!("Atomic move failed on device for {}", remote_target));
    }
    let final_sha = remote_sha256(adb, serial, remote_target)?
        .ok_or_else(|| format!("Target missing after move: {}", remote_target))?;
    if !final_sha.eq_ignore_ascii_case(expected_sha) {
        return Err(format!(
            "Final Android SHA-256 verification failed ({} != {})",
            final_sha, expected_sha
        ));
    }
    Ok(final_sha)
}

fn ensure_android_backup<L>(
    adb: &Path,
    serial: &str,
    remote: &str,
    bak_path: &Path,
    lang: PatchLang,
    en_sha: &str,
    id_sha: &str,
    label: &str,
    log: &mut L,
) -> Result<bool, String>
where
    L: FnMut(&str, LogLevel),
{
    let cur_sha = remote_sha256(adb, serial, remote)?
        .ok_or_else(|| format!("Android {} not found on device: {}", label, remote))?;

    if !bak_path.is_file() {
        let is_en = sha_matches(&cur_sha, en_sha);
        let is_id = sha_matches(&cur_sha, id_sha);
        if is_en || is_id {
            match lang {
                PatchLang::En if is_en => return Ok(true),
                PatchLang::Id if is_id => return Ok(true),
                PatchLang::En if is_id => {
                    return Err(format!(
                        "Android {} is patched in ID and original PC backup is missing. Restore Original JP first before switching to EN.",
                        label
                    ));
                }
                PatchLang::Id if is_en => {
                    return Err(format!(
                        "Android {} is patched in EN and original PC backup is missing. Restore Original JP first before switching to ID.",
                        label
                    ));
                }
                _ => {}
            }
        }
        log(
            &format!("  Pulling original Android {} for PC backup...", label),
            LogLevel::Info,
        );
        let tmp_bak = bak_path.with_extension("original.tmp");
        adb_pull(adb, serial, remote, &tmp_bak)?;
        let pulled_sha = sha256_file(&tmp_bak)?;
        if !pulled_sha.eq_ignore_ascii_case(&cur_sha) {
            let _ = fs::remove_file(&tmp_bak);
            return Err(format!(
                "Pulled backup SHA-256 mismatch for {} ({} != {})",
                label, pulled_sha, cur_sha
            ));
        }
        fs::rename(&tmp_bak, bak_path)
            .map_err(|e| format!("Failed finalizing backup {:?}: {}", bak_path, e))?;
        log(
            &format!(
                "  Saved verified original backup: {} (SHA256={}...)",
                bak_path.file_name().unwrap_or_default().to_string_lossy(),
                &pulled_sha[..12.min(pulled_sha.len())]
            ),
            LogLevel::Ok,
        );
        return Ok(false);
    }

    // Backup exists; check if remote is already on desired language
    let target_sha = match lang {
        PatchLang::En => en_sha,
        PatchLang::Id => id_sha,
    };
    if sha_matches(&cur_sha, target_sha) {
        return Ok(true);
    }
    Ok(false)
}

pub fn apply_android_patch<L, P>(
    adb: &Path,
    serial: &str,
    lang: PatchLang,
    mut log: L,
    mut on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_adb_server(adb)?;
    ensure_android_not_running(adb, serial, "patching")?;
    let bak_dir = android_backup_dir(serial);
    fs::create_dir_all(&bak_dir).map_err(|e| format!("Create Android backup dir failed: {}", e))?;

    let mut rewritten = 0usize;
    log(
        &format!(
            "=== Starting Android {} Patch on {} ===",
            lang.label(),
            serial
        ),
        LogLevel::Info,
    );
    on_progress(
        0,
        ANDROID_TOTAL_STRINGS,
        &format!("Checking Android {} via ADB...", serial),
    );

    // [1/2] Android VStringPool (74,055 strings)
    log(
        &format!("=== [1/2] Patching Android VStringPool ({}) ===", lang.short_code()),
        LogLevel::Info,
    );
    let vs_bak = bak_dir.join("1qnamndresrohzjhuvt2lw.r.original");
    on_progress(0, ANDROID_TOTAL_STRINGS, "Checking / backing up Android VStringPool (130 MB)...");
    let vs_already = ensure_android_backup(
        adb,
        serial,
        REMOTE_VSTRING,
        &vs_bak,
        lang,
        ANDROID_VSTRING_EN_SHA,
        ANDROID_VSTRING_ID_SHA,
        "VStringPool",
        &mut log,
    )?;
    if vs_already {
        log(
            &format!("  Android VStringPool already matches {} (74,055 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        rewritten += 74055;
        on_progress(rewritten, ANDROID_TOTAL_STRINGS, "Android VStringPool already up-to-date");
    } else {
        let orig_buf = fs::read(&vs_bak).map_err(|e| format!("Read Android VStringPool backup failed: {}", e))?;
        let (out_buf, count) = patch_engine::patch_android_vstring_bytes(
            &orig_buf,
            lang,
            &mut log,
            &mut |delta, stg| on_progress(delta, ANDROID_TOTAL_STRINGS, stg),
        )?;
        let staged = bak_dir.join("1qnamndresrohzjhuvt2lw.r.staged");
        fs::write(&staged, &out_buf).map_err(|e| format!("Write staged Android VStringPool failed: {}", e))?;
        let local_sha = sha256_file(&staged)?;
        on_progress(count, ANDROID_TOTAL_STRINGS, "Pushing patched VStringPool to Android (130 MB)...");
        log("  Pushing patched VStringPool to Android via ADB (atomic swap)...", LogLevel::Info);
        let push_res = push_atomic_verified(adb, serial, &staged, REMOTE_VSTRING, &local_sha);
        let _ = fs::remove_file(&staged);
        let final_sha = push_res?;
        log(
            &format!(
                "  Android VStringPool [{}] installed & verified! Rewritten={}, SHA256={}",
                lang.short_code(),
                count,
                final_sha
            ),
            LogLevel::Ok,
        );
        rewritten += count;
    }

    // [2/2] Android Static UI KeyConfig (3,135 strings)
    log(
        &format!("=== [2/2] Patching Android Static UI KeyConfig ({}) ===", lang.short_code()),
        LogLevel::Info,
    );
    let ui_bak = bak_dir.join("nibdvaxcnfh3en5bnvhtw.r.original");
    on_progress(rewritten, ANDROID_TOTAL_STRINGS, "Checking / backing up Android Static UI (6 MB)...");
    let ui_already = ensure_android_backup(
        adb,
        serial,
        REMOTE_STATIC_UI,
        &ui_bak,
        lang,
        ANDROID_STATIC_UI_EN_SHA,
        ANDROID_STATIC_UI_ID_SHA,
        "Static UI KeyConfig",
        &mut log,
    )?;
    if ui_already {
        log(
            &format!("  Android Static UI KeyConfig already matches {} (3,135 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        rewritten += 3135;
        on_progress(rewritten, ANDROID_TOTAL_STRINGS, "Android Static UI already up-to-date");
    } else {
        let orig_buf = fs::read(&ui_bak).map_err(|e| format!("Read Android Static UI backup failed: {}", e))?;
        let (out_buf, count) = patch_engine::patch_static_ui_bytes(
            &orig_buf,
            ANDROID_STATIC_UI_SUB_OFFSET,
            lang,
            &mut log,
            &mut |delta, stg| on_progress(rewritten + delta, ANDROID_TOTAL_STRINGS, stg),
        )?;
        let staged = bak_dir.join("nibdvaxcnfh3en5bnvhtw.r.staged");
        fs::write(&staged, &out_buf).map_err(|e| format!("Write staged Android Static UI failed: {}", e))?;
        let local_sha = sha256_file(&staged)?;
        on_progress(
            rewritten + count,
            ANDROID_TOTAL_STRINGS,
            "Pushing patched Static UI to Android (6 MB)...",
        );
        log("  Pushing patched Static UI to Android via ADB (atomic swap)...", LogLevel::Info);
        let push_res = push_atomic_verified(adb, serial, &staged, REMOTE_STATIC_UI, &local_sha);
        let _ = fs::remove_file(&staged);
        let final_sha = push_res?;
        log(
            &format!(
                "  Android Static UI KeyConfig [{}] installed & verified! Rewritten={}, SHA256={}",
                lang.short_code(),
                count,
                final_sha
            ),
            LogLevel::Ok,
        );
        rewritten += count;
    }

    on_progress(
        rewritten,
        ANDROID_TOTAL_STRINGS,
        &format!("Android patch verified 100% ({})!", lang.short_code()),
    );
    log(
        &format!(
            "=== ANDROID COMPLETE [{}]! Rewritten & verified: {} strings ===",
            lang.label(),
            rewritten
        ),
        LogLevel::Ok,
    );
    Ok(rewritten)
}

pub fn patch_android_news<L, P>(
    adb: &Path,
    serial: &str,
    lang: PatchLang,
    mut log: L,
    mut on_progress: P,
) -> Result<(usize, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_adb_server(adb)?;
    ensure_android_not_running(adb, serial, "translating news")?;
    let news_work = android_backup_dir(serial).join("NewsWork");
    let local_hc = news_work.join("HTTPCache");
    fs::create_dir_all(&local_hc).map_err(|e| format!("Create local News staging dir failed: {}", e))?;

    let remote_lib = format!("{}/Library", REMOTE_FILES_ROOT);
    let remote_hc = format!("{}/HTTPCache", REMOTE_FILES_ROOT);

    // Check remote Library size
    let sz_out = adb_shell(
        adb,
        serial,
        &format!("stat -c %s '{}' 2>/dev/null || echo 0", remote_lib),
    )?;
    let lib_sz = sz_out.trim().parse::<u64>().unwrap_or(0);
    if lib_sz <= 16 {
        return Err(
            "Android News cache is empty. Open the in-game News/Announcements screen once on Android, close the app, then click Translate News again."
                .into(),
        );
    }

    on_progress(0, 16, "Pulling Android HTTP News cache via ADB...");
    log("  Pulling Android Library & HTTPCache files...", LogLevel::Info);
    adb_pull(adb, serial, &remote_lib, &news_work.join("Library"))?;

    let list_out = adb_shell(adb, serial, &format!("ls '{}' 2>/dev/null || true", remote_hc))?;
    let mut remote_entries: Vec<String> = list_out
        .split_whitespace()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .map(|s| s.to_string())
        .collect();
    remote_entries.sort();
    for entry in &remote_entries {
        let rpath = format!("{}/{}", remote_hc, entry);
        let lpath = local_hc.join(entry);
        let _ = adb_pull(adb, serial, &rpath, &lpath);
    }

    let (patched, total) = patch_engine::patch_news_cache_for_selftest(
        &news_work,
        lang,
        &mut log,
        &mut on_progress,
    )?;

    on_progress(patched, total.max(1), "Pushing translated Android News cache...");
    let lib_sha = sha256_file(&news_work.join("Library"))?;
    push_atomic_verified(adb, serial, &news_work.join("Library"), &remote_lib, &lib_sha)?;

    if let Ok(rd) = fs::read_dir(&local_hc) {
        for ent in rd.flatten() {
            if !ent.path().is_file() {
                continue;
            }
            let fname = ent.file_name().to_string_lossy().to_string();
            let fsha = sha256_file(&ent.path())?;
            let rdst = format!("{}/{}", remote_hc, fname);
            push_atomic_verified(adb, serial, &ent.path(), &rdst, &fsha)?;
        }
    }

    log(
        &format!(
            "=== ANDROID NEWS COMPLETE [{}]! Translated {}/{} cached announcements ===",
            lang.short_code(),
            patched,
            total
        ),
        LogLevel::Ok,
    );
    Ok((patched, total))
}

pub fn rollback_android_device<L, P>(
    adb: &Path,
    serial: &str,
    mut log: L,
    mut on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_adb_server(adb)?;
    ensure_android_not_running(adb, serial, "rollback")?;
    let bak_dir = android_backup_dir(serial);
    let specs = [
        ("VStringPool", REMOTE_VSTRING, "1qnamndresrohzjhuvt2lw.r.original"),
        ("Static UI KeyConfig", REMOTE_STATIC_UI, "nibdvaxcnfh3en5bnvhtw.r.original"),
    ];
    let total = specs.len();
    let mut restored = 0usize;
    log(
        &format!("=== Starting Android Rollback to Original JP on {} ===", serial),
        LogLevel::Warn,
    );

    for (i, (name, remote, bak_name)) in specs.iter().enumerate() {
        on_progress(i, total, &format!("Restoring Android {}...", name));
        let bak = bak_dir.join(bak_name);
        if !bak.is_file() {
            log(
                &format!("  [SKIP] No PC backup found for Android {} ({})", name, bak_name),
                LogLevel::Warn,
            );
            continue;
        }
        let bak_sha = sha256_file(&bak)?;
        let cur_sha = remote_sha256(adb, serial, remote)?.unwrap_or_default();
        if cur_sha.eq_ignore_ascii_case(&bak_sha) {
            log(&format!("  [OK] Android {} is already Original JP.", name), LogLevel::Ok);
            restored += 1;
            continue;
        }
        let post_sha = push_atomic_verified(adb, serial, &bak, remote, &bak_sha)?;
        log(
            &format!(
                "  [RESTORED] Android {} -> Original JP (SHA256={}...)",
                name,
                &post_sha[..16.min(post_sha.len())]
            ),
            LogLevel::Ok,
        );
        restored += 1;
    }

    // Restore News cache if backed up locally
    let news_work = bak_dir.join("NewsWork");
    let news_bak = news_work.join("_NewsPatchBackup");
    if news_bak.join("Library").is_file() {
        if let Ok(n) = patch_engine::rollback_news_cache(&news_work, &mut log) {
            if n > 0 {
                let remote_lib = format!("{}/Library", REMOTE_FILES_ROOT);
                let remote_hc = format!("{}/HTTPCache", REMOTE_FILES_ROOT);
                if let Ok(lib_sha) = sha256_file(&news_work.join("Library")) {
                    let _ = push_atomic_verified(
                        adb,
                        serial,
                        &news_work.join("Library"),
                        &remote_lib,
                        &lib_sha,
                    );
                }
                if let Ok(rd) = fs::read_dir(news_work.join("HTTPCache")) {
                    for ent in rd.flatten() {
                        if ent.path().is_file() {
                            let fname = ent.file_name().to_string_lossy().to_string();
                            if let Ok(fsha) = sha256_file(&ent.path()) {
                                let _ = push_atomic_verified(
                                    adb,
                                    serial,
                                    &ent.path(),
                                    &format!("{}/{}", remote_hc, fname),
                                    &fsha,
                                );
                            }
                        }
                    }
                }
                log(
                    &format!("  [RESTORED] Android Cached News ({} announcements) -> Original JP", n),
                    LogLevel::Ok,
                );
            }
        }
    }

    on_progress(
        total,
        total,
        &format!("Android rollback complete ({}/{} restored).", restored, total),
    );
    log(
        &format!(
            "=== Android Rollback complete! Restored {}/{} targets to Original JP ===",
            restored, total
        ),
        LogLevel::Ok,
    );
    Ok(restored)
}
