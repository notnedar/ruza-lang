use crate::crypto_lz4::{read_u32_le, sha256_file, xor_mmap_in_place};
use crate::patch_engine::{
    self, BundlePatchState, ClientVersionInfo, LogLevel, PatchLang, PatchStateFile, TargetStatus,
    ANDROID_EXPECTED_RES_REV, ANDROID_EXPECTED_VERSION, ANDROID_PACKAGE_ID,
    ANDROID_STATIC_UI_KNOWN_EN_SHAS, ANDROID_STATIC_UI_KNOWN_ID_SHAS, ANDROID_STATIC_UI_ORIG_SHAS,
    ANDROID_TOTAL_STRINGS, ANDROID_VSTRING_KNOWN_EN_SHAS, ANDROID_VSTRING_KNOWN_ID_SHAS,
    ANDROID_VSTRING_ORIG_SHAS, EMBED_MMAP_KEY, KNOWN_ANDROID_STATIC_UI_NAMES, STATIC_UI_STRINGS,
    VSTRING_STRINGS,
};
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x08000000;

pub const REMOTE_FILES_ROOT: &str = "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files";
pub const REMOTE_RAW_BUNDLE_DIR: &str =
    "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/raw_base_package/BundleFiles";
pub const REMOTE_DEFAULT_BUNDLE_DIR: &str =
    "/sdcard/Android/data/com.Yostar.AzurpromiliaJP/files/.res/default_package/BundleFiles";
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

fn base64_decode(input: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity((input.len() * 3) / 4);
    let mut vals = [0u8; 4];
    let mut n = 0usize;
    for b in input.bytes() {
        let v = match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            b'=' => {
                if n == 2 {
                    out.push((vals[0] << 2) | (vals[1] >> 4));
                } else if n == 3 {
                    out.push((vals[0] << 2) | (vals[1] >> 4));
                    out.push((vals[1] << 4) | (vals[2] >> 2));
                }
                break;
            }
            _ => continue,
        };
        vals[n] = v;
        n += 1;
        if n == 4 {
            out.push((vals[0] << 2) | (vals[1] >> 4));
            out.push((vals[1] << 4) | (vals[2] >> 2));
            out.push((vals[2] << 6) | vals[3]);
            n = 0;
        }
    }
    out
}

fn adb_read_slice(adb: &Path, serial: &str, remote_path: &str, offset: usize, len: usize) -> Option<Vec<u8>> {
    let q = offset / 4096;
    let r = offset % 4096;
    let blocks = ((r + len) + 4095) / 4096;
    let cmd = format!(
        "dd if='{}' bs=4096 skip={} count={} 2>/dev/null | base64",
        remote_path, q, blocks
    );
    let b64 = adb_shell(adb, serial, &cmd).ok()?;
    let decoded = base64_decode(&b64);
    if decoded.len() < r + len {
        return None;
    }
    Some(decoded[r..r + len].to_vec())
}

fn is_safe_bundle_name(name: &str) -> bool {
    !name.is_empty()
        && name.ends_with(".r")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

struct RemoteVString {
    file_name: String,
    remote_path: String,
    entry_count: usize,
}

struct RemoteStaticUi {
    file_name: String,
    remote_path: String,
    sub_offset: usize,
}

fn discover_android_vstring_adb(adb: &Path, serial: &str) -> Result<Option<RemoteVString>, String> {
    let cmd = format!(
        "for f in '{}'/*.r; do [ -f \"$f\" ] || continue; sz=$(stat -c %s \"$f\" 2>/dev/null || echo 0); if [ \"$sz\" -ge 50000000 ]; then mt=$(stat -c %Y \"$f\" 2>/dev/null || echo 0); echo \"$sz $mt $f\"; fi; done",
        REMOTE_RAW_BUNDLE_DIR
    );
    let out = adb_shell(adb, serial, &cmd)?;
    let mut best: Option<(usize, u64, String, String)> = None;
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        let Some(sz_s) = parts.next() else { continue };
        let Some(mt_s) = parts.next() else { continue };
        let Some(rpath) = parts.next() else { continue };
        let Ok(fsize) = sz_s.parse::<usize>() else { continue };
        let mt = mt_s.parse::<u64>().unwrap_or(0);
        let fname = rpath.rsplit('/').next().unwrap_or("");
        if !is_safe_bundle_name(fname) {
            continue;
        }
        let Some(mut h1) = adb_read_slice(adb, serial, rpath, 4120, 24) else { continue };
        xor_mmap_in_place(&mut h1, 4120, EMBED_MMAP_KEY);
        let hash_bytes = read_u32_le(&h1, 0) as usize;
        let hash_start = read_u32_le(&h1, 4) as usize;
        if hash_start != 4120 {
            continue;
        }
        let Some(str_sec_off) = 4120usize.checked_add(hash_bytes) else { continue };
        if str_sec_off + 20 > fsize {
            continue;
        }
        let Some(mut sh) = adb_read_slice(adb, serial, rpath, str_sec_off, 20) else { continue };
        xor_mmap_in_place(&mut sh, str_sec_off, EMBED_MMAP_KEY);
        let count = read_u32_le(&sh, 16) as usize;
        if count < 500_000 || str_sec_off + 20 + count.saturating_mul(4) >= fsize {
            continue;
        }
        let better = match &best {
            None => true,
            Some((bc, bm, _, _)) => count > *bc || (count == *bc && mt > *bm),
        };
        if better {
            best = Some((count, mt, fname.to_string(), rpath.to_string()));
        }
    }
    Ok(best.map(|(entry_count, _, file_name, remote_path)| RemoteVString {
        file_name,
        remote_path,
        entry_count,
    }))
}

fn probe_remote_static_ui_file(
    adb: &Path,
    serial: &str,
    bak_dir: &Path,
    fname: &str,
    state_file: &PatchStateFile,
) -> Result<Option<RemoteStaticUi>, String> {
    if !is_safe_bundle_name(fname) {
        return Ok(None);
    }
    let remote_path = format!("{}/{}", REMOTE_DEFAULT_BUNDLE_DIR, fname);
    let sz_out = adb_shell(adb, serial, &format!("stat -c %s '{}' 2>/dev/null || echo 0", remote_path))?;
    let rsize = sz_out.trim().parse::<u64>().unwrap_or(0);
    if rsize < 100_000 {
        return Ok(None);
    }
    // Fast-path 1: cached filename + sub_offset in _ruza_state.json (zero pull!)
    if state_file.static_ui.file_name == fname && state_file.static_ui.sub_offset > 0 {
        return Ok(Some(RemoteStaticUi {
            file_name: fname.to_string(),
            remote_path,
            sub_offset: state_file.static_ui.sub_offset,
        }));
    }
    // Fast-path 2: local original backup exists (zero pull!)
    let local_bak = bak_dir.join(format!("{}.original", fname));
    if local_bak.is_file() {
        if let Ok(buf) = fs::read(&local_bak) {
            if let Some((sub_offset, _)) = patch_engine::find_static_ui_subbundle_in_bytes(&buf) {
                return Ok(Some(RemoteStaticUi {
                    file_name: fname.to_string(),
                    remote_path,
                    sub_offset,
                }));
            }
        }
    }
    // Pull to temp file and inspect UnityCN CAB header
    let tmp = bak_dir.join("_probe_static_ui.tmp");
    if adb_pull(adb, serial, &remote_path, &tmp).is_ok() {
        let res = fs::read(&tmp)
            .ok()
            .and_then(|buf| patch_engine::find_static_ui_subbundle_in_bytes(&buf));
        let _ = fs::remove_file(&tmp);
        if let Some((sub_offset, _)) = res {
            return Ok(Some(RemoteStaticUi {
                file_name: fname.to_string(),
                remote_path,
                sub_offset,
            }));
        }
    }
    let _ = fs::remove_file(&tmp);
    Ok(None)
}

fn discover_android_static_ui_adb(
    adb: &Path,
    serial: &str,
    bak_dir: &Path,
    state_file: &mut PatchStateFile,
) -> Result<Option<RemoteStaticUi>, String> {
    let mut checked: HashSet<String> = HashSet::new();

    // Tier 1: cached name + known Android Static UI names
    let mut fast_names: Vec<String> = Vec::new();
    if !state_file.static_ui.file_name.is_empty() {
        fast_names.push(state_file.static_ui.file_name.clone());
    }
    for &k in KNOWN_ANDROID_STATIC_UI_NAMES {
        fast_names.push(k.to_string());
    }
    for fname in fast_names {
        if !checked.insert(fname.clone()) {
            continue;
        }
        if let Some(found) = probe_remote_static_ui_file(adb, serial, bak_dir, &fname, state_file)? {
            if state_file.static_ui.file_name != found.file_name
                || state_file.static_ui.sub_offset != found.sub_offset
            {
                state_file.static_ui.file_name = found.file_name.clone();
                state_file.static_ui.sub_offset = found.sub_offset;
                patch_engine::save_patch_state(bak_dir, state_file);
            }
            return Ok(Some(found));
        }
    }

    // Tier 2/3: list default_package/BundleFiles/*.r (2.5-10 MB newest-first, then 100 KB-30 MB)
    let cmd = format!(
        "for f in '{}'/*.r; do [ -f \"$f\" ] || continue; sz=$(stat -c %s \"$f\" 2>/dev/null || echo 0); if [ \"$sz\" -ge 100000 ] && [ \"$sz\" -le 30000000 ]; then mt=$(stat -c %Y \"$f\" 2>/dev/null || echo 0); echo \"$sz $mt $f\"; fi; done",
        REMOTE_DEFAULT_BUNDLE_DIR
    );
    let out = adb_shell(adb, serial, &cmd)?;
    let mut cands: Vec<(bool, u64, String)> = Vec::new();
    for line in out.lines() {
        let mut parts = line.split_whitespace();
        let Some(sz_s) = parts.next() else { continue };
        let Some(mt_s) = parts.next() else { continue };
        let Some(rpath) = parts.next() else { continue };
        let Ok(sz) = sz_s.parse::<u64>() else { continue };
        let mt = mt_s.parse::<u64>().unwrap_or(0);
        let fname = rpath.rsplit('/').next().unwrap_or("");
        if !is_safe_bundle_name(fname) || checked.contains(fname) {
            continue;
        }
        let in_sweet_spot = (2_500_000..=10_000_000).contains(&sz);
        cands.push((!in_sweet_spot, u64::MAX - mt, fname.to_string()));
    }
    cands.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    for (_, _, fname) in cands {
        if let Some(found) = probe_remote_static_ui_file(adb, serial, bak_dir, &fname, state_file)? {
            state_file.static_ui.file_name = found.file_name.clone();
            state_file.static_ui.sub_offset = found.sub_offset;
            patch_engine::save_patch_state(bak_dir, state_file);
            return Ok(Some(found));
        }
    }
    Ok(None)
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
    let mut state_file = patch_engine::load_patch_state(&bak_dir);
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

    let disc_vs = discover_android_vstring_adb(adb, serial)?;
    let disc_ui = discover_android_static_ui_adb(adb, serial, &bak_dir, &mut state_file)?;

    let mut targets = Vec::with_capacity(2);

    // [1/2] VStringPool
    match disc_vs {
        Some(vs) => {
            let bak_name = format!("{}.original", vs.file_name);
            let bak = bak_dir.join(&bak_name);
            let has_backup = bak.is_file();
            let sha = remote_sha256(adb, serial, &vs.remote_path)?.unwrap_or_default();
            let state = patch_engine::classify_bundle_state(
                &vs.file_name,
                &sha,
                &bak,
                &state_file.vstring,
                ANDROID_VSTRING_ORIG_SHAS,
                ANDROID_VSTRING_KNOWN_EN_SHAS,
                ANDROID_VSTRING_KNOWN_ID_SHAS,
                true,
            );
            let disp_cnt = if state_file.vstring.file_name == vs.file_name && state_file.vstring.matched_count > 0 {
                state_file.vstring.matched_count
            } else {
                VSTRING_STRINGS
            };
            let state_lbl = match state.as_str() {
                "patched_en" => "ENGLISH",
                "patched_id" => "INDONESIAN",
                "patched_custom" => "CUSTOM",
                "original" => "ORIGINAL JP",
                _ => "MODIFIED",
            };
            let lvl = if state.starts_with("patched_") { LogLevel::Ok } else { LogLevel::Info };
            log(
                &format!(
                    "  [ANDROID {}] VStringPool [{}] ({} strings, pool={}, backup={})",
                    state_lbl,
                    vs.file_name,
                    disp_cnt,
                    vs.entry_count,
                    if has_backup { "YES" } else { "NO" }
                ),
                lvl,
            );
            targets.push(TargetStatus {
                id: "vstring".into(),
                name: "VStringPool".into(),
                subtitle: "Story, Quests, Skills, Kibo, Settings".into(),
                rel_path: vs.remote_path,
                backup_name: bak_name,
                string_count: disp_cnt,
                state,
                has_backup,
                sha256: sha,
            });
        }
        None => {
            log("  [MISSING] Android VStringPool", LogLevel::Warn);
            targets.push(TargetStatus {
                id: "vstring".into(),
                name: "VStringPool".into(),
                subtitle: "Story, Quests, Skills, Kibo, Settings".into(),
                rel_path: REMOTE_RAW_BUNDLE_DIR.into(),
                backup_name: String::new(),
                string_count: VSTRING_STRINGS,
                state: "missing".into(),
                has_backup: false,
                sha256: String::new(),
            });
        }
    }

    // [2/2] Static UI KeyConfig
    match disc_ui {
        Some(ui) => {
            let bak_name = format!("{}.original", ui.file_name);
            let bak = bak_dir.join(&bak_name);
            let has_backup = bak.is_file();
            let sha = remote_sha256(adb, serial, &ui.remote_path)?.unwrap_or_default();
            let state = patch_engine::classify_bundle_state(
                &ui.file_name,
                &sha,
                &bak,
                &state_file.static_ui,
                ANDROID_STATIC_UI_ORIG_SHAS,
                ANDROID_STATIC_UI_KNOWN_EN_SHAS,
                ANDROID_STATIC_UI_KNOWN_ID_SHAS,
                true,
            );
            let state_lbl = match state.as_str() {
                "patched_en" => "ENGLISH",
                "patched_id" => "INDONESIAN",
                "patched_custom" => "CUSTOM",
                "original" => "ORIGINAL JP",
                _ => "MODIFIED",
            };
            let lvl = if state.starts_with("patched_") { LogLevel::Ok } else { LogLevel::Info };
            log(
                &format!(
                    "  [ANDROID {}] Static UI KeyConfig [{}] ({} strings, off={}, backup={})",
                    state_lbl,
                    ui.file_name,
                    STATIC_UI_STRINGS,
                    ui.sub_offset,
                    if has_backup { "YES" } else { "NO" }
                ),
                lvl,
            );
            targets.push(TargetStatus {
                id: "static_ui".into(),
                name: "Static UI KeyConfig".into(),
                subtitle: "Prefab Buttons, Headers, Notices".into(),
                rel_path: ui.remote_path,
                backup_name: bak_name,
                string_count: STATIC_UI_STRINGS,
                state,
                has_backup,
                sha256: sha,
            });
        }
        None => {
            log("  [MISSING] Android Static UI KeyConfig", LogLevel::Warn);
            targets.push(TargetStatus {
                id: "static_ui".into(),
                name: "Static UI KeyConfig".into(),
                subtitle: "Prefab Buttons, Headers, Notices".into(),
                rel_path: REMOTE_DEFAULT_BUNDLE_DIR.into(),
                backup_name: String::new(),
                string_count: STATIC_UI_STRINGS,
                state: "missing".into(),
                has_backup: false,
                sha256: String::new(),
            });
        }
    }

    let mut reasons = Vec::new();
    if !detected_version.is_empty() && detected_version != ANDROID_EXPECTED_VERSION {
        reasons.push(format!(
            "Android version {} != {}",
            detected_version, ANDROID_EXPECTED_VERSION
        ));
    }
    let modified_cnt = targets
        .iter()
        .filter(|t| t.state == "modified" && !t.has_backup)
        .count();
    let missing_cnt = targets.iter().filter(|t| t.state == "missing").count();
    // Smart hotfix-proof sync: only complain about res_rev if bundles are actually missing!
    if missing_cnt > 0 && !detected_res_rev.is_empty() && detected_res_rev != ANDROID_EXPECTED_RES_REV {
        reasons.push(format!(
            "Android resource revision r{} != r{}",
            detected_res_rev, ANDROID_EXPECTED_RES_REV
        ));
    }
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

fn ensure_android_backup_v2<L>(
    adb: &Path,
    serial: &str,
    file_name: &str,
    remote: &str,
    bak_path: &Path,
    lang: PatchLang,
    custom: Option<&patch_engine::CustomPack>,
    state_entry: &BundlePatchState,
    known_en: &[&str],
    known_id: &[&str],
    label: &str,
    log: &mut L,
) -> Result<(bool, String), String>
where
    L: FnMut(&str, LogLevel),
{
    let cur_sha = remote_sha256(adb, serial, remote)?
        .ok_or_else(|| format!("Android {} not found on device: {}", label, remote))?;

    let st_match = state_entry.file_name == file_name
        && !state_entry.patched_sha.is_empty()
        && cur_sha.eq_ignore_ascii_case(&state_entry.patched_sha);

    if !bak_path.is_file() {
        let is_en = patch_engine::sha_in_list(&cur_sha, known_en)
            || (st_match && state_entry.lang == "patched_en");
        let is_id = patch_engine::sha_in_list(&cur_sha, known_id)
            || (st_match && state_entry.lang == "patched_id");
        let is_custom = st_match && state_entry.lang == "patched_custom";
        if is_en || is_id || is_custom {
            if custom.is_some() {
                return Err(format!(
                    "Android {} is already patched and original PC backup is missing. Restore Original JP first before applying a Custom Language Pack.",
                    label
                ));
            }
            if st_match && ((lang == PatchLang::En && is_en) || (lang == PatchLang::Id && is_id)) {
                return Ok((true, state_entry.orig_sha.clone()));
            }
            return Err(format!(
                "Android {} is already patched and original PC backup is missing. Restore Original JP first.",
                label
            ));
        }
        log(
            &format!("  Pulling original Android {} [{}] for PC backup...", label, file_name),
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
        return Ok((false, pulled_sha));
    }

    let orig_sha = if state_entry.file_name == file_name && !state_entry.orig_sha.is_empty() {
        state_entry.orig_sha.clone()
    } else {
        sha256_file(bak_path)?
    };

    if custom.is_some() {
        return Ok((false, orig_sha));
    }
    let want_tag = match lang {
        PatchLang::En => "patched_en",
        PatchLang::Id => "patched_id",
    };
    if st_match && state_entry.lang == want_tag {
        return Ok((true, orig_sha));
    }
    Ok((false, orig_sha))
}

pub fn apply_android_patch<L, P>(
    adb: &Path,
    serial: &str,
    lang: PatchLang,
    custom: Option<&patch_engine::CustomPack>,
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
    let mut state_file = patch_engine::load_patch_state(&bak_dir);

    let active_label = match custom {
        Some(c) => format!("{} ({} · base={})", c.meta.name, c.meta.code, c.base.as_str().to_uppercase()),
        None => lang.label().to_string(),
    };
    let active_code = match custom {
        Some(c) => c.meta.code.as_str(),
        None => lang.short_code(),
    };

    let mut rewritten = 0usize;
    log(
        &format!(
            "=== Starting Android {} Patch on {} ===",
            active_label,
            serial
        ),
        LogLevel::Info,
    );
    if let Some(c) = custom {
        log(
            &format!(
                "  Custom Pack: {} ({} vstring + {} static_ui overrides)",
                c.meta.file_name, c.meta.vstring_count, c.meta.static_ui_count
            ),
            LogLevel::Info,
        );
    }
    on_progress(
        0,
        ANDROID_TOTAL_STRINGS,
        &format!("Discovering active Android bundles on {}...", serial),
    );

    let disc_vs = discover_android_vstring_adb(adb, serial)?
        .ok_or("Android VStringPool bundle not found on device. Finish in-game download first.")?;
    let disc_ui = discover_android_static_ui_adb(adb, serial, &bak_dir, &mut state_file)?
        .ok_or("Android Static UI bundle not found on device. Finish in-game download first.")?;

    // [1/2] Android VStringPool (universal hash-keyed!)
    log(
        &format!("=== [1/2] Patching Android VStringPool [{} · {}] ===", disc_vs.file_name, active_code),
        LogLevel::Info,
    );
    let vs_bak = bak_dir.join(format!("{}.original", disc_vs.file_name));
    on_progress(0, ANDROID_TOTAL_STRINGS, "Checking / backing up Android VStringPool...");
    let (vs_already, vs_orig_sha) = ensure_android_backup_v2(
        adb,
        serial,
        &disc_vs.file_name,
        &disc_vs.remote_path,
        &vs_bak,
        lang,
        custom,
        &state_file.vstring,
        ANDROID_VSTRING_KNOWN_EN_SHAS,
        ANDROID_VSTRING_KNOWN_ID_SHAS,
        "VStringPool",
        &mut log,
    )?;
    if vs_already {
        let cnt = state_file.vstring.matched_count.max(VSTRING_STRINGS);
        log(
            &format!("  Android VStringPool [{}] already matches {} ({} strings).", disc_vs.file_name, active_code, cnt),
            LogLevel::Ok,
        );
        rewritten += cnt;
        on_progress(rewritten, ANDROID_TOTAL_STRINGS, "Android VStringPool already up-to-date");
    } else {
        let orig_buf = fs::read(&vs_bak).map_err(|e| format!("Read Android VStringPool backup failed: {}", e))?;
        let (out_buf, count) = patch_engine::patch_android_vstring_bytes(
            &orig_buf,
            lang,
            custom,
            &mut log,
            &mut |delta, stg| on_progress(delta, ANDROID_TOTAL_STRINGS, stg),
        )?;
        let staged = bak_dir.join(format!("{}.staged", disc_vs.file_name));
        fs::write(&staged, &out_buf).map_err(|e| format!("Write staged Android VStringPool failed: {}", e))?;
        let local_sha = sha256_file(&staged)?;
        on_progress(count, ANDROID_TOTAL_STRINGS, "Pushing patched VStringPool to Android...");
        log("  Pushing patched VStringPool to Android via ADB (atomic swap)...", LogLevel::Info);
        let push_res = push_atomic_verified(adb, serial, &staged, &disc_vs.remote_path, &local_sha);
        let _ = fs::remove_file(&staged);
        let final_sha = push_res?;
        state_file.vstring = BundlePatchState {
            file_name: disc_vs.file_name.clone(),
            sub_offset: 0,
            lang: patch_engine::state_tag_for(lang, custom),
            custom_code: custom.map(|c| c.meta.code.clone()).unwrap_or_default(),
            orig_sha: vs_orig_sha,
            patched_sha: final_sha.clone(),
            matched_count: count,
        };
        patch_engine::save_patch_state(&bak_dir, &state_file);
        log(
            &format!(
                "  Android VStringPool [{} · {}] installed & verified! Rewritten={}, SHA256={}",
                disc_vs.file_name,
                active_code,
                count,
                final_sha
            ),
            LogLevel::Ok,
        );
        rewritten += count;
    }

    // [2/2] Android Static UI KeyConfig (dynamically discovered!)
    log(
        &format!("=== [2/2] Patching Android Static UI [{} · {}] ===", disc_ui.file_name, active_code),
        LogLevel::Info,
    );
    let ui_bak = bak_dir.join(format!("{}.original", disc_ui.file_name));
    on_progress(rewritten, ANDROID_TOTAL_STRINGS, "Checking / backing up Android Static UI...");
    let (ui_already, ui_orig_sha) = ensure_android_backup_v2(
        adb,
        serial,
        &disc_ui.file_name,
        &disc_ui.remote_path,
        &ui_bak,
        lang,
        custom,
        &state_file.static_ui,
        ANDROID_STATIC_UI_KNOWN_EN_SHAS,
        ANDROID_STATIC_UI_KNOWN_ID_SHAS,
        "Static UI KeyConfig",
        &mut log,
    )?;
    if ui_already {
        log(
            &format!("  Android Static UI [{}] already matches {} ({} strings).", disc_ui.file_name, active_code, STATIC_UI_STRINGS),
            LogLevel::Ok,
        );
        rewritten += STATIC_UI_STRINGS;
        on_progress(rewritten, ANDROID_TOTAL_STRINGS, "Android Static UI already up-to-date");
    } else {
        let orig_buf = fs::read(&ui_bak).map_err(|e| format!("Read Android Static UI backup failed: {}", e))?;
        let sub_off = patch_engine::find_static_ui_subbundle_in_bytes(&orig_buf)
            .map(|(off, _)| off)
            .unwrap_or(disc_ui.sub_offset);
        let (out_buf, count) = patch_engine::patch_static_ui_bytes(
            &orig_buf,
            sub_off,
            lang,
            custom,
            &mut log,
            &mut |delta, stg| on_progress(rewritten + delta, ANDROID_TOTAL_STRINGS, stg),
        )?;
        let staged = bak_dir.join(format!("{}.staged", disc_ui.file_name));
        fs::write(&staged, &out_buf).map_err(|e| format!("Write staged Android Static UI failed: {}", e))?;
        let local_sha = sha256_file(&staged)?;
        on_progress(
            rewritten + count,
            ANDROID_TOTAL_STRINGS,
            "Pushing patched Static UI to Android...",
        );
        log("  Pushing patched Static UI to Android via ADB (atomic swap)...", LogLevel::Info);
        let push_res = push_atomic_verified(adb, serial, &staged, &disc_ui.remote_path, &local_sha);
        let _ = fs::remove_file(&staged);
        let final_sha = push_res?;
        state_file.static_ui = BundlePatchState {
            file_name: disc_ui.file_name.clone(),
            sub_offset: sub_off,
            lang: patch_engine::state_tag_for(lang, custom),
            custom_code: custom.map(|c| c.meta.code.clone()).unwrap_or_default(),
            orig_sha: ui_orig_sha,
            patched_sha: final_sha.clone(),
            matched_count: count,
        };
        patch_engine::save_patch_state(&bak_dir, &state_file);
        log(
            &format!(
                "  Android Static UI KeyConfig [{} · {}] installed & verified! Rewritten={}, SHA256={}",
                disc_ui.file_name,
                active_code,
                count,
                final_sha
            ),
            LogLevel::Ok,
        );
        rewritten += count;
    }

    on_progress(
        rewritten,
        rewritten.max(ANDROID_TOTAL_STRINGS),
        &format!("Android patch verified 100% ({})!", active_code),
    );
    log(
        &format!(
            "=== ANDROID COMPLETE [{}]! Rewritten & verified: {} strings ===",
            active_label,
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
    let mut state_file = patch_engine::load_patch_state(&bak_dir);
    let disc_vs = discover_android_vstring_adb(adb, serial)?;
    let disc_ui = discover_android_static_ui_adb(adb, serial, &bak_dir, &mut state_file)?;

    let vs_fname = disc_vs
        .as_ref()
        .map(|v| v.file_name.clone())
        .or_else(|| {
            if !state_file.vstring.file_name.is_empty() {
                Some(state_file.vstring.file_name.clone())
            } else {
                Some("1qnamndresrohzjhuvt2lw.r".to_string())
            }
        })
        .unwrap();
    let ui_fname = disc_ui
        .as_ref()
        .map(|u| u.file_name.clone())
        .or_else(|| {
            if !state_file.static_ui.file_name.is_empty() {
                Some(state_file.static_ui.file_name.clone())
            } else {
                Some("nibdvaxcnfh3en5bnvhtw.r".to_string())
            }
        })
        .unwrap();

    let specs = [
        (
            "vstring",
            "VStringPool",
            format!("{}/{}", REMOTE_RAW_BUNDLE_DIR, vs_fname),
            format!("{}.original", vs_fname),
        ),
        (
            "static_ui",
            "Static UI KeyConfig",
            format!("{}/{}", REMOTE_DEFAULT_BUNDLE_DIR, ui_fname),
            format!("{}.original", ui_fname),
        ),
    ];
    let total = specs.len();
    let mut restored = 0usize;
    log(
        &format!("=== Starting Android Rollback to Original JP on {} ===", serial),
        LogLevel::Warn,
    );

    for (i, (id, name, remote, bak_name)) in specs.iter().enumerate() {
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
        if !cur_sha.eq_ignore_ascii_case(&bak_sha) {
            let post_sha = push_atomic_verified(adb, serial, &bak, remote, &bak_sha)?;
            log(
                &format!(
                    "  [RESTORED] Android {} -> Original JP (SHA256={}...)",
                    name,
                    &post_sha[..16.min(post_sha.len())]
                ),
                LogLevel::Ok,
            );
        } else {
            log(&format!("  [OK] Android {} is already Original JP.", name), LogLevel::Ok);
        }
        let st = if *id == "vstring" {
            &mut state_file.vstring
        } else {
            &mut state_file.static_ui
        };
        st.lang = "original".into();
        st.custom_code.clear();
        st.patched_sha.clear();
        st.orig_sha = bak_sha;
        restored += 1;
    }
    patch_engine::save_patch_state(&bak_dir, &state_file);

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
