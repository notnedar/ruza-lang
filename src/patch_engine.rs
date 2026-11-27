use crate::crypto_lz4::*;
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use sysinfo::{ProcessesToUpdate, System};

// Embedded verified payloads compiled directly into the standalone executable
const EMBED_MMAP_KEY: &[u8] = include_bytes!("../resources/mmap_key_4096.bin");

const EMBED_VSTRING_EN_GZ: &[u8] = include_bytes!("../resources/vstring_en.bin.gz");
const EMBED_STATIC_UI_EN_GZ: &[u8] = include_bytes!("../resources/static_ui_en.bin.gz");
const EMBED_BUILTIN_EN_GZ: &[u8] = include_bytes!("../resources/builtin_en.bin.gz");
const EMBED_RESOURCES_DIFF_EN_GZ: &[u8] = include_bytes!("../resources/resources_diff.bin.gz");

const EMBED_VSTRING_ID_GZ: &[u8] = include_bytes!("../resources/vstring_id.bin.gz");
const EMBED_STATIC_UI_ID_GZ: &[u8] = include_bytes!("../resources/static_ui_id.bin.gz");
const EMBED_BUILTIN_ID_GZ: &[u8] = include_bytes!("../resources/builtin_id.bin.gz");
const EMBED_RESOURCES_DIFF_ID_GZ: &[u8] = include_bytes!("../resources/resources_diff_id.bin.gz");

const EMBED_NEWS_EN_GZ: &[u8] = include_bytes!("../resources/news_en.bin.gz");
const EMBED_NEWS_ID_GZ: &[u8] = include_bytes!("../resources/news_id.bin.gz");

pub const TOTAL_STRINGS: usize = 74055 + 3135 + 113 + 358; // 77,661

pub const VSTRING_REL: &str = r"AzurPromiliaJP_Data\.res\raw_base_package\BundleFiles\z5xzl3cg6ao0ilpbzykexg.r";
pub const STATIC_UI_REL: &str = r"AzurPromiliaJP_Data\.res\default_package\BundleFiles\yr49bxop0f3reujfsmyfca.r";
pub const STATIC_PKG_REL: &str = r"AzurPromiliaJP_Data\StreamingAssets\.res\static_package\6pq4a7hlp7zidu91dvvww.r";
pub const RESOURCES_REL: &str = r"AzurPromiliaJP_Data\resources.assets";

// Verified Original JP SHA-256 hashes
pub const VSTRING_ORIG_SHA: &str = "99B8129D49586D26745630A76423609123DDCB1A7AAD3E0A971886B13336A878";
pub const STATIC_UI_ORIG_SHA: &str = "701F04842F5156DA177BD4946A1CF9B7C055F2A614003C2C13C8567FA14E0FC3";
pub const STATIC_PKG_ORIG_SHA: &str = "C7C80B98B263F161583A2AB52A5B7039636ACBABCAC8C365F9F37224D11BC4B3";
pub const RESOURCES_ORIG_SHA: &str = "DF93CE7192D6A46B33350204349E66A45E410124E3057224AD1A17E6F24B115B";

// Verified English (EN) SHA-256 hashes (v0.2.1 UI Fit Pass)
pub const VSTRING_EN_SHA: &str = "D57E7F499EF6737FF7AB0F957921C626D689CF6A5D612966C2483BC28D54F1CB";
pub const STATIC_UI_EN_SHA: &str = "40B7950BB70692BF0EBE2B536B7A202EA993DEDE0481E262762DE3A27AE449F9";
pub const STATIC_UI_EN_SHA_ALT: &str = "";
pub const STATIC_PKG_EN_SHA: &str = "7FEB394BD2FEAC1F8FC862A1254CD954EDC1D49544DE285D3E6D9C914755F33E";
pub const RESOURCES_EN_SHA: &str = "C95E1A2C5C5977E105022B49266D3E2E32173A0E1F4E75BD807C7A93A9299883";

// Verified Bahasa Indonesia (ID) SHA-256 hashes (v0.2.1 UI Fit Pass)
pub const VSTRING_ID_SHA: &str = "B968C8F97791B704CC50877C1B388F9C81C022B285A9244BEDD2ED9B59EB7AC3";
pub const STATIC_UI_ID_SHA: &str = "8E69653B7EEC783B5235036374601BF248A2DF5A76337BA8DBC58CB5803FBF0B";
pub const STATIC_PKG_ID_SHA: &str = "3EBE582B61D995108E1DA1F333D102B6218A5C9D33E03C931CB7EEEE585DC1FC";
pub const RESOURCES_ID_SHA: &str = "85FADFAAC9708C845803FA438C5C2AA634A798FAE48405C9FC40953B4A7772A2";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PatchLang {
    En,
    Id,
}

impl PatchLang {
    pub fn from_opt(s: Option<&str>) -> Self {
        match s.map(|v| v.to_ascii_lowercase()).as_deref() {
            Some("id") => PatchLang::Id,
            _ => PatchLang::En,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PatchLang::En => "English (EN)",
            PatchLang::Id => "Bahasa Indonesia (ID)",
        }
    }

    pub fn short_code(self) -> &'static str {
        match self {
            PatchLang::En => "EN",
            PatchLang::Id => "ID",
        }
    }
}

#[derive(Clone, Serialize)]
pub struct ProcessInfo {
    pub name: String,
    pub pid: u32,
}

#[derive(Clone, Serialize)]
pub struct TargetStatus {
    pub id: String,
    pub name: String,
    pub subtitle: String,
    pub rel_path: String,
    pub backup_name: String,
    pub string_count: usize,
    pub state: String, // "patched_en" | "patched_id" | "original" | "modified" | "missing"
    pub has_backup: bool,
    pub sha256: String,
}

#[derive(Clone, Copy)]
pub enum LogLevel {
    Info,
    Ok,
    Warn,
    Error,
    Diff,
}

impl LogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LogLevel::Info => "info",
            LogLevel::Ok => "ok",
            LogLevel::Warn => "warn",
            LogLevel::Error => "err",
            LogLevel::Diff => "diff",
        }
    }
}

pub fn find_running_game_processes() -> Vec<ProcessInfo> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    let watched = ["azurpromiliajp.exe", "azupro_jp_gamelauncher.exe"];
    let mut out = Vec::new();
    for (pid, proc_) in sys.processes() {
        let name = proc_.name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if watched.iter().any(|w| lower == *w) {
            out.push(ProcessInfo {
                name,
                pid: pid.as_u32(),
            });
        }
    }
    out
}

pub fn is_valid_game_folder(dir: &Path) -> bool {
    dir.join("AzurPromiliaJP.exe").is_file() && dir.join("AzurPromiliaJP_Data").is_dir()
}

pub fn auto_detect_game_folder() -> String {
    let mut candidates: Vec<PathBuf> = vec![
        PathBuf::from(r"E:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"C:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"D:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"F:\Program Files (x86)\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"C:\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"D:\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"E:\YostarGames\AZUPRO_JP"),
        PathBuf::from(r"D:\Games\AZUPRO_JP"),
        PathBuf::from(r"E:\Games\AZUPRO_JP"),
    ];
    for drive in b'C'..=b'Z' {
        let root = format!("{}:\\", drive as char);
        if Path::new(&root).exists() {
            candidates.push(PathBuf::from(&root).join(r"Program Files (x86)\YostarGames\AZUPRO_JP"));
            candidates.push(PathBuf::from(&root).join(r"Program Files\YostarGames\AZUPRO_JP"));
            candidates.push(PathBuf::from(&root).join(r"YostarGames\AZUPRO_JP"));
        }
    }
    for c in candidates {
        if is_valid_game_folder(&c) {
            return c.to_string_lossy().to_string();
        }
    }
    String::new()
}

fn sha_matches(sha: &str, expected: &str) -> bool {
    !expected.is_empty() && sha.eq_ignore_ascii_case(expected)
}

pub fn scan_game_folder<F>(game_dir: &Path, mut log: F) -> Vec<TargetStatus>
where
    F: FnMut(&str, LogLevel),
{
    let bak_dir = game_dir.join("_ENPatchBackup");
    let specs = [
        (
            "vstring",
            "VStringPool",
            "Story, Quests, Skills, Kibo, Settings",
            VSTRING_REL,
            "z5xzl3cg6ao0ilpbzykexg.r.original",
            74055usize,
            VSTRING_ORIG_SHA,
            VSTRING_EN_SHA,
            "",
            VSTRING_ID_SHA,
        ),
        (
            "static_ui",
            "Static UI KeyConfig",
            "Prefab Buttons, Headers, Notices",
            STATIC_UI_REL,
            "yr49bxop0f3reujfsmyfca.r.original",
            3135usize,
            STATIC_UI_ORIG_SHA,
            STATIC_UI_EN_SHA,
            STATIC_UI_EN_SHA_ALT,
            STATIC_UI_ID_SHA,
        ),
        (
            "static_pkg",
            "StaticPackage CSV",
            "Boot & Updater Messages",
            STATIC_PKG_REL,
            "6pq4a7hlp7zidu91dvvww.r.original",
            113usize,
            STATIC_PKG_ORIG_SHA,
            STATIC_PKG_EN_SHA,
            "",
            STATIC_PKG_ID_SHA,
        ),
        (
            "resources",
            "Resources Assets",
            "Fallback Startup & SDK",
            RESOURCES_REL,
            "resources.assets.original",
            358usize,
            RESOURCES_ORIG_SHA,
            RESOURCES_EN_SHA,
            "",
            RESOURCES_ID_SHA,
        ),
    ];

    let mut out = Vec::with_capacity(4);
    for (id, name, sub, rel, bak_name, count, orig_sha, en_sha, en_sha_alt, id_sha) in specs {
        let full = game_dir.join(rel);
        let bak = bak_dir.join(bak_name);
        let has_backup = bak.is_file();
        if !full.is_file() {
            log(&format!("  [MISSING] {}", rel), LogLevel::Warn);
            out.push(TargetStatus {
                id: id.into(),
                name: name.into(),
                subtitle: sub.into(),
                rel_path: rel.into(),
                backup_name: bak_name.into(),
                string_count: count,
                state: "missing".into(),
                has_backup,
                sha256: String::new(),
            });
            continue;
        }
        let sha = sha256_file(&full).unwrap_or_default();
        let state = if sha_matches(&sha, id_sha) {
            "patched_id"
        } else if sha_matches(&sha, en_sha) || sha_matches(&sha, en_sha_alt) {
            "patched_en"
        } else if sha_matches(&sha, orig_sha) {
            "original"
        } else {
            "modified"
        };
        let lvl = if state == "patched_en" || state == "patched_id" {
            LogLevel::Ok
        } else {
            LogLevel::Info
        };
        let state_lbl = match state {
            "patched_en" => "ENGLISH",
            "patched_id" => "INDONESIAN",
            "original" => "ORIGINAL JP",
            "modified" => "MODIFIED",
            _ => "MISSING",
        };
        log(
            &format!(
                "  [{}] {} ({} strings, backup={})",
                state_lbl,
                name,
                count,
                if has_backup { "YES" } else { "NO" }
            ),
            lvl,
        );
        out.push(TargetStatus {
            id: id.into(),
            name: name.into(),
            subtitle: sub.into(),
            rel_path: rel.into(),
            backup_name: bak_name.into(),
            string_count: count,
            state: state.into(),
            has_backup,
            sha256: sha,
        });
    }
    out
}

fn ensure_backup<F>(
    target: &Path,
    bak: &Path,
    en_sha: &str,
    en_sha_alt: &str,
    id_sha: &str,
    log: &mut F,
) -> Result<(), String>
where
    F: FnMut(&str, LogLevel),
{
    if bak.is_file() {
        return Ok(());
    }
    let cur_sha = sha256_file(target)?;
    if sha_matches(&cur_sha, en_sha) || sha_matches(&cur_sha, en_sha_alt) || sha_matches(&cur_sha, id_sha) {
        log(
            "  Target is already patched; skipping backup overwrite so original backup is never polluted.",
            LogLevel::Warn,
        );
        return Ok(());
    }
    if let Some(parent) = bak.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Create backup dir failed: {}", e))?;
    }
    fs::copy(target, bak).map_err(|e| format!("Backup copy failed: {}", e))?;
    log(
        &format!(
            "  Saved verified original backup: {} (SHA256={}...)",
            bak.file_name().unwrap().to_string_lossy(),
            &cur_sha[..12.min(cur_sha.len())]
        ),
        LogLevel::Ok,
    );
    Ok(())
}

fn check_no_backup_switch_guard(
    target: &Path,
    bak: &Path,
    lang: PatchLang,
    en_sha: &str,
    en_sha_alt: &str,
    id_sha: &str,
) -> Result<bool, String> {
    if bak.is_file() {
        return Ok(false);
    }
    let cur = sha256_file(target)?;
    let is_en = sha_matches(&cur, en_sha) || sha_matches(&cur, en_sha_alt);
    let is_id = sha_matches(&cur, id_sha);
    match lang {
        PatchLang::En if is_en => Ok(true),
        PatchLang::Id if is_id => Ok(true),
        PatchLang::En if is_id => Err(
            "Target is currently patched in ID and _ENPatchBackup is missing. Restore Original JP files before switching to EN."
                .into(),
        ),
        PatchLang::Id if is_en => Err(
            "Target is currently patched in EN and _ENPatchBackup is missing. Restore Original JP files before switching to ID."
                .into(),
        ),
        _ => Ok(false),
    }
}

fn write_exclusive_verified(target: &Path, new_bytes: &[u8], backup_for_rollback: &Path) -> Result<(), String> {
    let prev_bytes = if backup_for_rollback.is_file() {
        fs::read(backup_for_rollback).map_err(|e| format!("Read rollback backup failed: {}", e))?
    } else {
        fs::read(target).map_err(|e| format!("Read current target failed: {}", e))?
    };

    // Exclusive open (share_mode = 0 => no concurrent readers/writers)
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(target)
        .map_err(|e| format!("Exclusive open {:?} failed (is the game open?): {}", target, e))?;

    let cur_len = f.metadata().map_err(|e| format!("Stat failed: {}", e))?.len() as usize;
    if cur_len != new_bytes.len() {
        return Err(format!(
            "Target size mismatch ({} != {})",
            cur_len,
            new_bytes.len()
        ));
    }

    let write_res = (|| -> Result<(), String> {
        f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
        f.write_all(new_bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
        Ok(())
    })();

    if let Err(err) = write_res {
        let _ = f.seek(SeekFrom::Start(0));
        let _ = f.write_all(&prev_bytes);
        let _ = f.sync_all();
        return Err(format!("Write failed (auto-rolled back safely): {}", err));
    }
    Ok(())
}

fn load_vstring_map(gz: &[u8]) -> Result<HashMap<usize, Vec<u8>>, String> {
    let raw = gunzip(gz)?;
    let count = read_u32_le(&raw, 0) as usize;
    let mut map = HashMap::with_capacity(count);
    let mut p = 4usize;
    for _ in 0..count {
        let id = read_u32_le(&raw, p) as usize;
        let len = read_u32_le(&raw, p + 4) as usize;
        p += 8;
        map.insert(id, raw[p..p + len].to_vec());
        p += len;
    }
    Ok(map)
}

fn load_string_dict(gz: &[u8]) -> Result<HashMap<String, String>, String> {
    let raw = gunzip(gz)?;
    let count = read_u32_le(&raw, 0) as usize;
    let mut map = HashMap::with_capacity(count);
    let mut p = 4usize;
    for _ in 0..count {
        let klen = (raw[p] as usize) | ((raw[p + 1] as usize) << 8);
        p += 2;
        let k = String::from_utf8_lossy(&raw[p..p + klen]).into_owned();
        p += klen;
        let vlen = read_u32_le(&raw, p) as usize;
        p += 4;
        let v = String::from_utf8_lossy(&raw[p..p + vlen]).into_owned();
        p += vlen;
        map.insert(k, v);
    }
    Ok(map)
}

fn ensure_no_running_game_processes(action: &str) -> Result<(), String> {
    let procs = find_running_game_processes();
    if !procs.is_empty() {
        let s = procs
            .iter()
            .map(|p| format!("{} (PID {})", p.name, p.pid))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!("Blocked: {} is currently running. Close it before {}!", s, action));
    }
    Ok(())
}

pub fn apply_patch<L, P>(
    game_dir: &Path,
    lang: PatchLang,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_no_running_game_processes("patching")?;
    apply_patch_inner(game_dir, lang, log, on_progress)
}

pub fn apply_patch_for_selftest<L, P>(
    game_dir: &Path,
    lang: PatchLang,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    apply_patch_inner(game_dir, lang, log, on_progress)
}

fn apply_patch_inner<L, P>(
    game_dir: &Path,
    lang: PatchLang,
    mut log: L,
    mut on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    let bak_dir = game_dir.join("_ENPatchBackup");
    fs::create_dir_all(&bak_dir).map_err(|e| format!("Failed creating backup dir: {}", e))?;

    let mut rewritten = 0usize;
    log(
        &format!("=== Starting {} Patch Installation ===", lang.label()),
        LogLevel::Info,
    );
    on_progress(0, TOTAL_STRINGS, &format!("Unpacking embedded {} payloads...", lang.short_code()));

    // [1/4] VStringPool (74,055 strings)
    log(
        &format!("=== [1/4] Patching VStringPool ({}) ===", lang.short_code()),
        LogLevel::Info,
    );
    rewritten += patch_vstring_pool(game_dir, &bak_dir, lang, &mut log, &mut |delta, stage| {
        on_progress(rewritten + delta, TOTAL_STRINGS, stage);
    })?;

    // [2/4] Static UI KeyConfig (3,135 strings)
    log(
        &format!("=== [2/4] Patching Static UI KeyConfig ({}) ===", lang.short_code()),
        LogLevel::Info,
    );
    rewritten += patch_static_ui(game_dir, &bak_dir, lang, &mut log, &mut |delta, stage| {
        on_progress(rewritten + delta, TOTAL_STRINGS, stage);
    })?;

    // [3/4] StaticPackage updatableBuiltInLangString (113 strings)
    log(
        &format!("=== [3/4] Patching Startup/Updater StaticPackage ({}) ===", lang.short_code()),
        LogLevel::Info,
    );
    rewritten += patch_static_pkg(game_dir, &bak_dir, lang, &mut log, &mut |delta, stage| {
        on_progress(rewritten + delta, TOTAL_STRINGS, stage);
    })?;

    // [4/4] resources.assets (358 strings)
    log(
        &format!("=== [4/4] Patching Startup/SDK Resources ({}) ===", lang.short_code()),
        LogLevel::Info,
    );
    rewritten += patch_resources_assets(game_dir, &bak_dir, lang, &mut log, &mut |delta, stage| {
        on_progress(rewritten + delta, TOTAL_STRINGS, stage);
    })?;

    on_progress(
        rewritten,
        TOTAL_STRINGS,
        &format!("All 4 targets verified 100% ({})!", lang.short_code()),
    );
    log(
        &format!(
            "=== COMPLETE [{}]! Rewritten & verified: {} strings ===",
            lang.label(),
            rewritten
        ),
        LogLevel::Ok,
    );
    Ok(rewritten)
}

fn patch_vstring_pool<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    lang: PatchLang,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let target = game_dir.join(VSTRING_REL);
    let bak = bak_dir.join("z5xzl3cg6ao0ilpbzykexg.r.original");
    if !target.is_file() {
        return Err(format!("VStringPool not found: {:?}", target));
    }
    ensure_backup(&target, &bak, VSTRING_EN_SHA, "", VSTRING_ID_SHA, log)?;
    if check_no_backup_switch_guard(&target, &bak, lang, VSTRING_EN_SHA, "", VSTRING_ID_SHA)? {
        log(
            &format!("  VStringPool already matches {} (74,055 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        on_sub(74055, "VStringPool already up-to-date");
        return Ok(74055);
    }
    let src_path = if bak.is_file() { &bak } else { &target };

    on_sub(0, &format!("Loading 74,055 VStringPool ({}) entries...", lang.short_code()));
    let gz = match lang {
        PatchLang::En => EMBED_VSTRING_EN_GZ,
        PatchLang::Id => EMBED_VSTRING_ID_GZ,
    };
    let translations = load_vstring_map(gz)?;
    log(
        &format!(
            "  Loaded {} embedded {} VStringPool translations.",
            translations.len(),
            lang.short_code()
        ),
        LogLevel::Info,
    );

    on_sub(0, "Decrypting VStringPool (133 MB MMap)...");
    let orig_buf = fs::read(src_path).map_err(|e| format!("Read VStringPool failed: {}", e))?;
    let mut h1 = orig_buf[4120..4144].to_vec();
    xor_mmap_in_place(&mut h1, 4120, EMBED_MMAP_KEY);
    let hash_bytes = read_u32_le(&h1, 0) as usize;
    let hash_start = read_u32_le(&h1, 4) as usize;
    if hash_start != 4120 {
        return Err(format!("Unexpected VStringHashWR startPos: {}", hash_start));
    }

    let str_sec_off = 4120 + hash_bytes;
    let mut str_buf = orig_buf[str_sec_off..].to_vec();
    xor_mmap_in_place(&mut str_buf, str_sec_off, EMBED_MMAP_KEY);
    let count = read_u32_le(&str_buf, 16) as usize;
    log(&format!("  Decrypted VStringPool: {} total strings.", count), LogLevel::Info);

    let mut new_str_buf = str_buf.clone();
    let spans = [("base_jp", 1495181usize, 1563711usize), ("hotfix_jp", 1658279usize, count - 1)];

    let mut rewritten = 0usize;
    let mut inplace_ids = Vec::new();
    for &id in translations.keys() {
        let in_span = (id >= 1495181 && id <= 1563711) || (id >= 1658279 && id < count);
        if !in_span {
            inplace_ids.push(id);
        }
    }

    for (name, start, end) in spans {
        let span_start_abs = read_u32_le(&str_buf, 20 + start * 4) as usize;
        let span_end_abs = if end + 1 < count {
            read_u32_le(&str_buf, 20 + (end + 1) * 4) as usize
        } else {
            orig_buf.len()
        };
        let capacity = span_end_abs - span_start_abs;
        let mut packed = Vec::with_capacity(capacity);
        let mut cursor = span_start_abs;

        for id in start..=end {
            let payload: &[u8] = if let Some(tr) = translations.get(&id) {
                rewritten += 1;
                if rewritten % 2500 == 0 {
                    on_sub(
                        rewritten,
                        &format!("Rewriting VStringPool [{} / {}]: {} / 74,055", lang.short_code(), name, rewritten),
                    );
                }
                tr.as_slice()
            } else {
                let o_abs = read_u32_le(&str_buf, 20 + id * 4) as usize;
                let o_rel = o_abs - str_sec_off;
                let o_len = read_i32_le(&str_buf, o_rel) as usize;
                &str_buf[o_rel + 4..o_rel + 4 + o_len]
            };
            write_u32_le(&mut new_str_buf, 20 + id * 4, cursor as u32);
            packed.extend_from_slice(&(payload.len() as i32).to_le_bytes());
            packed.extend_from_slice(payload);
            cursor += 4 + payload.len();
        }
        let used = cursor - span_start_abs;
        if used > capacity {
            return Err(format!("Span {} overflowed by {} bytes", name, used - capacity));
        }
        let dst_off = span_start_abs - str_sec_off;
        new_str_buf[dst_off..dst_off + used].copy_from_slice(&packed);
        if used < capacity {
            new_str_buf[dst_off + used..dst_off + capacity].fill(0);
        }
        log(
            &format!(
                "  Repacked {} [#{}..#{}]: {}/{} B (free slack {} B)",
                name, start, end, used, capacity, capacity - used
            ),
            LogLevel::Diff,
        );
    }

    for id in inplace_ids {
        let o_abs = read_u32_le(&str_buf, 20 + id * 4) as usize;
        let o_rel = o_abs - str_sec_off;
        let o_len = read_i32_le(&str_buf, o_rel) as usize;
        let payload = &translations[&id];
        if payload.len() > o_len {
            return Err(format!("In-place ID {} overflow", id));
        }
        write_i32_le(&mut new_str_buf, o_rel, payload.len() as i32);
        new_str_buf[o_rel + 4..o_rel + 4 + payload.len()].copy_from_slice(payload);
        if payload.len() < o_len {
            new_str_buf[o_rel + 4 + payload.len()..o_rel + 4 + o_len].fill(0);
        }
        rewritten += 1;
    }

    on_sub(rewritten, "Re-encrypting & verifying 74,055 VStringPool strings...");
    let mut out_buf = orig_buf.clone();
    out_buf[str_sec_off..].copy_from_slice(&new_str_buf);
    xor_mmap_in_place(&mut out_buf[str_sec_off..], str_sec_off, EMBED_MMAP_KEY);

    // Round-trip verification
    let mut v_sec = out_buf[str_sec_off..].to_vec();
    xor_mmap_in_place(&mut v_sec, str_sec_off, EMBED_MMAP_KEY);
    for (&id, expected) in &translations {
        let a = read_u32_le(&v_sec, 20 + id * 4) as usize;
        let rel = a - str_sec_off;
        let l = read_i32_le(&v_sec, rel) as usize;
        if l != expected.len() || &v_sec[rel + 4..rel + 4 + l] != expected.as_slice() {
            return Err(format!("VStringPool round-trip mismatch on #{}", id));
        }
    }

    write_exclusive_verified(&target, &out_buf, &bak)?;
    let post_sha = sha256_file(&target)?;
    log(
        &format!(
            "  VStringPool [{}] installed & verified! Rewritten={}, SHA256={}",
            lang.short_code(),
            rewritten,
            post_sha
        ),
        LogLevel::Ok,
    );
    on_sub(rewritten, "VStringPool verified (74,055 strings)");
    Ok(rewritten)
}

#[inline]
fn align4(n: usize) -> usize {
    (n + 3) & !3
}

fn patch_static_ui<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    lang: PatchLang,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    const SUB_OFFSET: usize = 420062;
    const SUB_SIZE: usize = 83151;
    const DATA_OFF: usize = 3392;
    const MB_NAME_OFF: usize = 3420;
    const OBJ0_SIZE_OFF: usize = 3220;
    const ORIG_MB_SIZE: usize = 161392;

    let target = game_dir.join(STATIC_UI_REL);
    let bak = bak_dir.join("yr49bxop0f3reujfsmyfca.r.original");
    if !target.is_file() {
        return Err(format!("Static UI bundle not found: {:?}", target));
    }
    ensure_backup(&target, &bak, STATIC_UI_EN_SHA, STATIC_UI_EN_SHA_ALT, STATIC_UI_ID_SHA, log)?;
    if check_no_backup_switch_guard(&target, &bak, lang, STATIC_UI_EN_SHA, STATIC_UI_EN_SHA_ALT, STATIC_UI_ID_SHA)? {
        log(
            &format!("  Static UI KeyConfig already matches {} (3,135 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        on_sub(3135, "Static UI already up-to-date");
        return Ok(3135);
    }
    let src_path = if bak.is_file() { &bak } else { &target };

    let gz = match lang {
        PatchLang::En => EMBED_STATIC_UI_EN_GZ,
        PatchLang::Id => EMBED_STATIC_UI_ID_GZ,
    };
    let tr_map = load_string_dict(gz)?;
    let orig_buf = fs::read(src_path).map_err(|e| format!("Read Static UI failed: {}", e))?;
    on_sub(0, "Decrypting UnityCN ast_staticTextKeyConfig.asset...");
    let bundle = UnityCnBundle::parse(&orig_buf, SUB_OFFSET)?;
    let orig_sf = bundle.read_all()?;

    let mut cur = MB_NAME_OFF + 28;
    let lang_len = read_i32_le(&orig_sf, cur) as usize;
    cur += 4;
    let lang_tag = String::from_utf8_lossy(&orig_sf[cur..cur + lang_len]).into_owned();
    cur = align4(cur + lang_len);
    let pair_count = read_i32_le(&orig_sf, cur) as usize;
    cur += 4;

    let mut new_mb = Vec::with_capacity(ORIG_MB_SIZE);
    new_mb.extend_from_slice(&orig_sf[DATA_OFF..MB_NAME_OFF + 28]);
    let write_str = |dst: &mut Vec<u8>, s: &str| {
        let b = s.as_bytes();
        dst.extend_from_slice(&(b.len() as i32).to_le_bytes());
        dst.extend_from_slice(b);
        let pad = align4(b.len()) - b.len();
        for _ in 0..pad {
            dst.push(0);
        }
    };
    write_str(&mut new_mb, &lang_tag);
    new_mb.extend_from_slice(&(pair_count as i32).to_le_bytes());

    let mut replaced = 0usize;
    for _ in 0..pair_count {
        let kl = read_i32_le(&orig_sf, cur) as usize;
        cur += 4;
        let k = String::from_utf8_lossy(&orig_sf[cur..cur + kl]).into_owned();
        cur = align4(cur + kl);
        let vl = read_i32_le(&orig_sf, cur) as usize;
        cur += 4;
        let orig_v = String::from_utf8_lossy(&orig_sf[cur..cur + vl]).into_owned();
        cur = align4(cur + vl);

        let v = if let Some(tr) = tr_map.get(&k) {
            replaced += 1;
            if replaced % 400 == 0 {
                on_sub(
                    replaced,
                    &format!("Rewriting Static UI KeyConfig [{}]: {} / 3,135", lang.short_code(), replaced),
                );
            }
            tr.as_str()
        } else {
            orig_v.as_str()
        };
        write_str(&mut new_mb, &k);
        write_str(&mut new_mb, v);
    }

    if new_mb.len() > ORIG_MB_SIZE {
        return Err(format!("Rebuilt Static UI exceeds slot ({} > {})", new_mb.len(), ORIG_MB_SIZE));
    }

    let mut new_sf = orig_sf.clone();
    new_sf[DATA_OFF..DATA_OFF + ORIG_MB_SIZE].fill(0);
    new_sf[DATA_OFF..DATA_OFF + new_mb.len()].copy_from_slice(&new_mb);
    write_u32_le(&mut new_sf, OBJ0_SIZE_OFF, new_mb.len() as u32);

    on_sub(replaced, "Compressing (LZ4HC exact) & UnityCN encrypting Static UI...");
    let u0 = bundle.blocks[0].u_size;
    let u1 = bundle.blocks[1].u_size;
    let total_budget = SUB_SIZE - bundle.data_start;
    let mut c0 = lz4_compress(&new_sf[..u0], 1024);
    let mut c1 = lz4_compress_exact(&new_sf[u0..u0 + u1], total_budget - c0.len(), 1024)?;
    bundle.cipher.encrypt_block_in_place(&mut c0, 0);
    bundle.cipher.encrypt_block_in_place(&mut c1, 1);

    let mut new_bi_plain = Vec::with_capacity(128);
    new_bi_plain.extend_from_slice(&bundle.data_hash);
    new_bi_plain.extend_from_slice(&2u32.to_be_bytes());
    for (u_sz, c_sz, fl) in [
        (u0, c0.len(), bundle.blocks[0].flags),
        (u1, c1.len(), bundle.blocks[1].flags),
    ] {
        new_bi_plain.extend_from_slice(&(u_sz as u32).to_be_bytes());
        new_bi_plain.extend_from_slice(&(c_sz as u32).to_be_bytes());
        new_bi_plain.extend_from_slice(&fl.to_be_bytes());
    }
    new_bi_plain.extend_from_slice(&(bundle.dirs.len() as u32).to_be_bytes());
    for d in &bundle.dirs {
        new_bi_plain.extend_from_slice(&d.offset.to_be_bytes());
        new_bi_plain.extend_from_slice(&d.size.to_be_bytes());
        new_bi_plain.extend_from_slice(&d.flags.to_be_bytes());
        new_bi_plain.extend_from_slice(d.name.as_bytes());
        new_bi_plain.push(0);
    }
    let new_bi_comp = lz4_compress(&new_bi_plain, 256);
    if bundle.blocks_info_off + new_bi_comp.len() > bundle.data_start {
        return Err("Compressed blocksInfo overflowed data_start".into());
    }

    let mut new_sub = vec![0u8; SUB_SIZE];
    new_sub[..bundle.header_prefix.len()].copy_from_slice(&bundle.header_prefix);
    write_i64_be(&mut new_sub, bundle.fsize_off, SUB_SIZE as i64);
    write_u32_be(&mut new_sub, bundle.cinfo_off, new_bi_comp.len() as u32);
    write_u32_be(&mut new_sub, bundle.cinfo_off + 4, new_bi_plain.len() as u32);
    new_sub[bundle.blocks_info_off..bundle.blocks_info_off + new_bi_comp.len()].copy_from_slice(&new_bi_comp);
    new_sub[bundle.data_start..bundle.data_start + c0.len()].copy_from_slice(&c0);
    new_sub[bundle.data_start + c0.len()..bundle.data_start + c0.len() + c1.len()].copy_from_slice(&c1);

    let mut out_buf = orig_buf.clone();
    out_buf[SUB_OFFSET..SUB_OFFSET + SUB_SIZE].copy_from_slice(&new_sub);

    // Round-trip verify decrypted SerializedFile
    let v_bundle = UnityCnBundle::parse(&out_buf, SUB_OFFSET)?;
    let v_sf = v_bundle.read_all()?;
    if v_sf != new_sf {
        return Err("Static UI round-trip verification failed".into());
    }

    write_exclusive_verified(&target, &out_buf, &bak)?;
    let post_sha = sha256_file(&target)?;
    log(
        &format!(
            "  Static UI KeyConfig [{}] installed & verified! Rewritten={}, SHA256={}",
            lang.short_code(),
            replaced,
            post_sha
        ),
        LogLevel::Ok,
    );
    on_sub(replaced, "Static UI KeyConfig verified (3,135 strings)");
    Ok(replaced)
}

fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut in_q = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if in_q {
            if ch == '"' {
                if i + 1 < chars.len() && chars[i + 1] == '"' {
                    field.push('"');
                    i += 1;
                } else {
                    in_q = false;
                }
            } else {
                field.push(ch);
            }
        } else if ch == '"' {
            in_q = true;
        } else if ch == ',' {
            row.push(std::mem::take(&mut field));
        } else if ch == '\n' {
            row.push(std::mem::take(&mut field));
            rows.push(std::mem::take(&mut row));
        } else if ch != '\r' {
            field.push(ch);
        }
        i += 1;
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows
}

fn serialize_csv_field(s: &str) -> String {
    if s.contains('"') || s.contains(',') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| {
        let u = c as u32;
        (0x3040..=0x30FF).contains(&u) || (0x4E00..=0x9FFF).contains(&u)
    })
}

fn patch_static_pkg<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    lang: PatchLang,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let target = game_dir.join(STATIC_PKG_REL);
    let bak = bak_dir.join("6pq4a7hlp7zidu91dvvww.r.original");
    if !target.is_file() {
        return Err(format!("StaticPackage bundle not found: {:?}", target));
    }
    ensure_backup(&target, &bak, STATIC_PKG_EN_SHA, "", STATIC_PKG_ID_SHA, log)?;
    if check_no_backup_switch_guard(&target, &bak, lang, STATIC_PKG_EN_SHA, "", STATIC_PKG_ID_SHA)? {
        log(
            &format!("  Startup StaticPackage already matches {} (113 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        on_sub(113, "StaticPackage already up-to-date");
        return Ok(113);
    }
    let src_path = if bak.is_file() { &bak } else { &target };

    let builtin_dict = match lang {
        PatchLang::En => load_string_dict(EMBED_BUILTIN_EN_GZ)?,
        PatchLang::Id => load_string_dict(EMBED_BUILTIN_ID_GZ)?,
    };
    let orig_buf = fs::read(src_path).map_err(|e| format!("Read StaticPackage failed: {}", e))?;
    on_sub(0, "Decrypting StaticPackage Block #5 (updatableBuiltInLangString)...");
    let bundle = UnityCnBundle::parse(&orig_buf, 0)?;
    let mut blk5 = bundle.read_block(5)?;

    let tag = b"updatableBuiltInLangString";
    let tag_pos = blk5
        .windows(tag.len())
        .position(|w| w == tag)
        .ok_or("updatableBuiltInLangString not found in block 5")?;
    let len_off = align4(tag_pos + tag.len());
    let csv_byte_len = read_i32_le(&blk5, len_off) as usize;
    let csv_data_off = len_off + 4;
    let has_bom = blk5[csv_data_off..csv_data_off + 3] == [0xEF, 0xBB, 0xBF];
    let csv_text = String::from_utf8_lossy(
        &blk5[csv_data_off + if has_bom { 3 } else { 0 }..csv_data_off + csv_byte_len],
    )
    .into_owned();

    let mut rows = parse_csv(&csv_text);
    let cht_idx = rows[0].iter().position(|x| x == "cht");
    let kr_idx = rows[0].iter().position(|x| x == "kr");
    let en_idx = rows[0].iter().position(|x| x == "en").ok_or("No 'en' column")?;
    let jp_idx = rows[0].iter().position(|x| x == "jp").ok_or("No 'jp' column")?;
    let mut replaced = 0usize;
    for r in rows.iter_mut().skip(1) {
        if r.len() <= jp_idx || r[0].is_empty() {
            continue;
        }
        let val = match lang {
            PatchLang::En => {
                let mut v = r[en_idx].trim().to_string();
                if v.is_empty() || has_cjk(&v) {
                    if let Some(fb) = builtin_dict.get(&r[0]) {
                        v = fb.clone();
                    }
                }
                v
            }
            PatchLang::Id => {
                if let Some(id_val) = builtin_dict.get(&r[0]) {
                    id_val.clone()
                } else {
                    r[en_idx].trim().to_string()
                }
            }
        };
        if !val.is_empty() && !has_cjk(&val) {
            if lang == PatchLang::Id {
                r[en_idx] = val.clone();
                if let Some(ci) = cht_idx {
                    if ci < r.len() {
                        r[ci].clear();
                    }
                }
                if let Some(ki) = kr_idx {
                    if ki < r.len() {
                        r[ki].clear();
                    }
                }
            }
            r[jp_idx] = val;
            replaced += 1;
        }
    }

    let mut out_csv_str = String::new();
    for r in &rows {
        for (j, f) in r.iter().enumerate() {
            if j > 0 {
                out_csv_str.push(',');
            }
            out_csv_str.push_str(&serialize_csv_field(f));
        }
        out_csv_str.push('\n');
    }
    let body = out_csv_str.as_bytes();
    let total_len = body.len() + if has_bom { 3 } else { 0 };
    if total_len > csv_byte_len {
        return Err(format!("Rebuilt CSV larger than slot ({} > {})", total_len, csv_byte_len));
    }
    let mut new_csv = vec![0x0Au8; csv_byte_len];
    let wp = if has_bom {
        new_csv[0..3].copy_from_slice(&[0xEF, 0xBB, 0xBF]);
        3
    } else {
        0
    };
    new_csv[wp..wp + body.len()].copy_from_slice(body);
    blk5[csv_data_off..csv_data_off + csv_byte_len].copy_from_slice(&new_csv);

    on_sub(replaced, "Compressing (LZ4HC exact) & UnityCN encrypting Block #5...");
    let blk5_info = &bundle.blocks[5];
    let mut comp5 = lz4_compress_exact(&blk5, blk5_info.c_size, 1024)?;
    bundle.cipher.encrypt_block_in_place(&mut comp5, 5);

    let mut out_buf = orig_buf.clone();
    out_buf[blk5_info.file_offset..blk5_info.file_offset + comp5.len()].copy_from_slice(&comp5);

    // Round-trip verify Block #5
    let v_bundle = UnityCnBundle::parse(&out_buf, 0)?;
    let v_blk5 = v_bundle.read_block(5)?;
    if v_blk5 != blk5 {
        return Err("StaticPackage Block #5 round-trip verification failed".into());
    }

    write_exclusive_verified(&target, &out_buf, &bak)?;
    let post_sha = sha256_file(&target)?;
    log(
        &format!(
            "  Startup StaticPackage [{}] installed & verified! Rewritten={}, SHA256={}",
            lang.short_code(),
            replaced,
            post_sha
        ),
        LogLevel::Ok,
    );
    on_sub(replaced, "StaticPackage verified (113 strings)");
    Ok(replaced)
}

fn patch_resources_assets<L, P>(
    game_dir: &Path,
    bak_dir: &Path,
    lang: PatchLang,
    log: &mut L,
    on_sub: &mut P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, &str),
{
    let target = game_dir.join(RESOURCES_REL);
    let bak = bak_dir.join("resources.assets.original");
    if !target.is_file() {
        return Err(format!("resources.assets not found: {:?}", target));
    }
    ensure_backup(&target, &bak, RESOURCES_EN_SHA, "", RESOURCES_ID_SHA, log)?;
    if check_no_backup_switch_guard(&target, &bak, lang, RESOURCES_EN_SHA, "", RESOURCES_ID_SHA)? {
        log(
            &format!("  resources.assets already matches {} (358 strings).", lang.short_code()),
            LogLevel::Ok,
        );
        on_sub(358, "resources.assets already up-to-date");
        return Ok(358);
    }

    let src_path = if bak.is_file() { &bak } else { &target };
    let (diff_gz, expected_sha) = match lang {
        PatchLang::En => (EMBED_RESOURCES_DIFF_EN_GZ, RESOURCES_EN_SHA),
        PatchLang::Id => (EMBED_RESOURCES_DIFF_ID_GZ, RESOURCES_ID_SHA),
    };
    let diff = gunzip(diff_gz)?;
    let expected_size = read_u32_le(&diff, 0) as usize;
    let span_count = read_u32_le(&diff, 4) as usize;

    let mut out_buf = fs::read(src_path).map_err(|e| format!("Read resources.assets failed: {}", e))?;
    if out_buf.len() != expected_size {
        return Err(format!("resources.assets size mismatch ({} != {})", out_buf.len(), expected_size));
    }
    let mut p = 8usize;
    for _ in 0..span_count {
        let off = read_u32_le(&diff, p) as usize;
        let len = read_u32_le(&diff, p + 4) as usize;
        p += 8;
        out_buf[off..off + len].copy_from_slice(&diff[p..p + len]);
        p += len;
    }

    let cand_sha = sha256_bytes(&out_buf);
    if !expected_sha.is_empty() && !cand_sha.eq_ignore_ascii_case(expected_sha) {
        return Err(format!("resources.assets post-patch SHA256 mismatch: {}", cand_sha));
    }
    write_exclusive_verified(&target, &out_buf, &bak)?;
    log(
        &format!(
            "  resources.assets [{}] installed & verified! Rewritten=358, SHA256={}",
            lang.short_code(),
            cand_sha
        ),
        LogLevel::Ok,
    );
    on_sub(358, "resources.assets verified (358 strings)");
    Ok(358)
}

pub fn rollback_to_original<L, P>(
    game_dir: &Path,
    news_cache_dir: Option<&Path>,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_no_running_game_processes("rollback")?;
    rollback_to_original_inner(game_dir, news_cache_dir, log, on_progress)
}

pub fn rollback_to_original_for_selftest<L, P>(
    game_dir: &Path,
    news_cache_dir: Option<&Path>,
    log: L,
    on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    rollback_to_original_inner(game_dir, news_cache_dir, log, on_progress)
}

fn rollback_to_original_inner<L, P>(
    game_dir: &Path,
    news_cache_dir: Option<&Path>,
    mut log: L,
    mut on_progress: P,
) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    let bak_dir = game_dir.join("_ENPatchBackup");
    let targets = scan_game_folder(game_dir, |_, _| {});
    let total = targets.len();
    let mut restored = 0usize;
    log("=== Starting Rollback to Original JP ===", LogLevel::Warn);

    for (i, t) in targets.iter().enumerate() {
        let full = game_dir.join(&t.rel_path);
        let bak = bak_dir.join(&t.backup_name);
        on_progress(i, total, &format!("Restoring {}...", t.name));

        if !bak.is_file() {
            log(&format!("  [SKIP] No backup found for {} ({})", t.name, t.backup_name), LogLevel::Warn);
            continue;
        }
        if !full.is_file() {
            log(&format!("  [SKIP] Target missing: {}", t.rel_path), LogLevel::Warn);
            continue;
        }
        let bak_sha = sha256_file(&bak)?;
        let cur_sha = sha256_file(&full)?;
        if cur_sha.eq_ignore_ascii_case(&bak_sha) {
            log(&format!("  [OK] {} is already Original JP.", t.name), LogLevel::Ok);
            restored += 1;
            continue;
        }
        let bak_bytes = fs::read(&bak).map_err(|e| format!("Read backup failed: {}", e))?;
        write_exclusive_verified(&full, &bak_bytes, &full)?;
        let post_sha = sha256_file(&full)?;
        if !post_sha.eq_ignore_ascii_case(&bak_sha) {
            return Err(format!("Rollback SHA-256 verification failed for {}", t.name));
        }
        log(
            &format!("  [RESTORED] {} -> Original JP (SHA256={}...)", t.name, &post_sha[..16]),
            LogLevel::Ok,
        );
        restored += 1;
    }

    if let Some(cache_dir) = news_cache_dir {
        match rollback_news_cache(cache_dir, &mut log) {
            Ok(n) if n > 0 => {
                log(
                    &format!("  [RESTORED] Cached News ({} announcements) -> Original JP", n),
                    LogLevel::Ok,
                );
            }
            Ok(_) => {}
            Err(e) => {
                log(&format!("  [WARN] News cache rollback skipped: {}", e), LogLevel::Warn);
            }
        }
    }

    on_progress(total, total, &format!("Rollback complete ({}/{} restored).", restored, total));
    log(&format!("=== Rollback complete! Restored {}/{} targets to Original JP ===", restored, total), LogLevel::Ok);
    Ok(restored)
}

// ---------- Cached In-Game News / Announcements (BestHTTP HTTPCache + Library v3) ----------

pub fn default_news_cache_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("USERPROFILE")
        .ok_or_else(|| "USERPROFILE environment variable not found".to_string())?;
    Ok(PathBuf::from(home)
        .join("AppData")
        .join("LocalLow")
        .join("YostarJP")
        .join("AzurPromiliaJP"))
}

struct NewsLibEntry {
    hash: Option<String>,
    hex_name: String,
    body_len_off: usize,
    body_len: usize,
}

fn read_7bit_str(buf: &[u8], p: &mut usize) -> Result<String, String> {
    let mut len = 0usize;
    let mut shift = 0u32;
    loop {
        if *p >= buf.len() {
            return Err("Unexpected EOF while reading 7-bit string length".into());
        }
        let b = buf[*p];
        *p += 1;
        len |= ((b & 0x7F) as usize) << shift;
        if (b & 0x80) == 0 {
            break;
        }
        shift += 7;
        if shift > 28 {
            return Err("Corrupt 7-bit string length in Library".into());
        }
    }
    if *p + len > buf.len() {
        return Err("Unexpected EOF while reading 7-bit string payload".into());
    }
    let s = String::from_utf8_lossy(&buf[*p..*p + len]).into_owned();
    *p += len;
    Ok(s)
}

fn extract_announcement_hash(url: &str) -> Option<String> {
    let lower = url.to_ascii_lowercase();
    let marker = "announcement/prod/";
    let idx = lower.find(marker)?;
    let rest = &lower[idx + marker.len()..];
    let suffix = "/index.html";
    let end = rest.find(suffix)?;
    let hash = &rest[..end];
    if hash.len() == 32 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hash.to_string())
    } else {
        None
    }
}

fn parse_httpcache_library(buf: &[u8]) -> Result<Vec<NewsLibEntry>, String> {
    if buf.len() < 16 {
        return Err("Library file too small".into());
    }
    let ver = read_i32_le(buf, 0);
    if ver != 3 {
        return Err(format!("Unsupported HTTP cache Library version: {}", ver));
    }
    let cnt = read_i32_le(buf, 12) as usize;
    let mut p = 16usize;
    let mut out = Vec::with_capacity(cnt);
    for _ in 0..cnt {
        let url = read_7bit_str(buf, &mut p)?;
        if p + 8 + 4 + 17 + 8 > buf.len() {
            return Err("Library entry truncated before metadata".into());
        }
        p += 8; // LastAccess
        let body_len_off = p;
        let body_len = read_i32_le(buf, p) as usize;
        p += 4;
        p += 17; // ver3 reserved bytes
        let mapped_idx = u64::from_le_bytes([
            buf[p],
            buf[p + 1],
            buf[p + 2],
            buf[p + 3],
            buf[p + 4],
            buf[p + 5],
            buf[p + 6],
            buf[p + 7],
        ]);
        p += 8;
        let _etag = read_7bit_str(buf, &mut p)?;
        let _last_mod = read_7bit_str(buf, &mut p)?;
        if p + 41 > buf.len() {
            return Err("Library entry truncated in footer timestamps".into());
        }
        p += 41; // Expires(8) + Age(8) + MaxAge(8) + Date(8) + MustRevalidate(1) + Received(8)
        out.push(NewsLibEntry {
            hash: extract_announcement_hash(&url),
            hex_name: format!("{:X}", mapped_idx),
            body_len_off,
            body_len,
        });
    }
    if p != buf.len() {
        return Err(format!("Library parse length mismatch ({} != {})", p, buf.len()));
    }
    Ok(out)
}

fn find_http_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn update_http_content_length(hdr_bytes: &[u8], new_len: usize) -> Vec<u8> {
    let hdr_str = String::from_utf8_lossy(hdr_bytes);
    let is_gzip = hdr_str
        .lines()
        .any(|l| l.to_ascii_lowercase().starts_with("content-encoding:") && l.to_ascii_lowercase().contains("gzip"));
    if is_gzip {
        return hdr_bytes.to_vec();
    }
    let mut lines: Vec<String> = hdr_str.split("\r\n").map(|s| s.to_string()).collect();
    for line in &mut lines {
        if line.to_ascii_lowercase().starts_with("content-length:") {
            *line = format!("Content-Length: {}", new_len);
        }
    }
    lines.join("\r\n").into_bytes()
}

pub fn patch_news_cache<L, P>(
    cache_base: &Path,
    lang: PatchLang,
    log: L,
    on_progress: P,
) -> Result<(usize, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    ensure_no_running_game_processes("translating news")?;
    patch_news_cache_inner(cache_base, lang, log, on_progress)
}

pub fn patch_news_cache_for_selftest<L, P>(
    cache_base: &Path,
    lang: PatchLang,
    log: L,
    on_progress: P,
) -> Result<(usize, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    patch_news_cache_inner(cache_base, lang, log, on_progress)
}

fn patch_news_cache_inner<L, P>(
    cache_base: &Path,
    lang: PatchLang,
    mut log: L,
    mut on_progress: P,
) -> Result<(usize, usize), String>
where
    L: FnMut(&str, LogLevel),
    P: FnMut(usize, usize, &str),
{
    let lib_path = cache_base.join("Library");
    let hc_dir = cache_base.join("HTTPCache");
    if !lib_path.is_file() || !hc_dir.is_dir() {
        return Err(
            "News cache not found in AppData\\LocalLow. Open the in-game News/Announcements screen once, close the client, then click Translate News again."
                .into(),
        );
    }

    let dict = match lang {
        PatchLang::En => load_string_dict(EMBED_NEWS_EN_GZ)?,
        PatchLang::Id => load_string_dict(EMBED_NEWS_ID_GZ)?,
    };

    log(
        &format!("=== Starting Cached News Translation ({}) ===", lang.label()),
        LogLevel::Info,
    );
    let live_lib = fs::read(&lib_path).map_err(|e| format!("Read Library failed: {}", e))?;
    let entries = parse_httpcache_library(&live_lib)?;

    let matched_total = entries
        .iter()
        .filter(|e| e.hash.as_ref().map(|h| dict.contains_key(h)).unwrap_or(false))
        .count();
    if matched_total == 0 {
        return Err(
            "No matching cached announcement HTML pages found yet. Open the in-game Announcements once, close the client, and try again."
                .into(),
        );
    }

    // Save verified original backups before first patch (never overwrite existing backup)
    let bak_dir = cache_base.join("_NewsPatchBackup");
    let bak_hc = bak_dir.join("HTTPCache");
    let bak_lib = bak_dir.join("Library");
    if !bak_lib.is_file() {
        let any_jp = entries.iter().any(|e| {
            if e.hash.is_none() {
                return false;
            }
            if let Ok(raw) = fs::read(hc_dir.join(&e.hex_name)) {
                if let Some(he) = find_http_header_end(&raw) {
                    return has_cjk(&String::from_utf8_lossy(&raw[he + 4..]));
                }
            }
            false
        });
        if any_jp {
            fs::create_dir_all(&bak_hc).map_err(|e| format!("Create news backup dir failed: {}", e))?;
            fs::copy(&lib_path, &bak_lib).map_err(|e| format!("Backup Library failed: {}", e))?;
            for e in &entries {
                if e.hash.is_some() {
                    let src = hc_dir.join(&e.hex_name);
                    let dst = bak_hc.join(&e.hex_name);
                    if src.is_file() && !dst.is_file() {
                        let _ = fs::copy(&src, &dst);
                    }
                }
            }
            log("  Saved verified original news cache backup in _NewsPatchBackup\\.", LogLevel::Ok);
        }
    }

    let mut new_lib = live_lib.clone();
    let mut patched = 0usize;
    for e in &entries {
        let Some(hash) = e.hash.as_ref() else { continue };
        let Some(html) = dict.get(hash) else { continue };

        let bak_file = bak_hc.join(&e.hex_name);
        let live_file = hc_dir.join(&e.hex_name);
        let src_file = if bak_file.is_file() { &bak_file } else { &live_file };
        if !src_file.is_file() {
            continue;
        }
        let raw = fs::read(src_file).map_err(|err| format!("Read HC/{} failed: {}", e.hex_name, err))?;
        let hdr_end = find_http_header_end(&raw)
            .ok_or_else(|| format!("Missing HTTP header terminator in HC/{}", e.hex_name))?;

        let new_body = html.as_bytes();
        let new_hdr = update_http_content_length(&raw[..hdr_end], new_body.len());
        let mut out_buf = Vec::with_capacity(new_hdr.len() + 4 + new_body.len());
        out_buf.extend_from_slice(&new_hdr);
        out_buf.extend_from_slice(b"\r\n\r\n");
        out_buf.extend_from_slice(new_body);

        fs::write(&live_file, &out_buf)
            .map_err(|err| format!("Write HC/{} failed: {}", e.hex_name, err))?;
        write_i32_le(&mut new_lib, e.body_len_off, new_body.len() as i32);
        patched += 1;
        log(
            &format!(
                "  [{}] HC/{} ({}): {} B -> {} B",
                lang.short_code(),
                e.hex_name,
                &hash[..8],
                e.body_len,
                new_body.len()
            ),
            LogLevel::Diff,
        );
        on_progress(
            patched,
            matched_total,
            &format!("Translating News [{}]: {} / {}", lang.short_code(), patched, matched_total),
        );
    }

    fs::write(&lib_path, &new_lib).map_err(|e| format!("Write Library failed: {}", e))?;

    // Round-trip verify updated Library + HTTPCache body lengths
    let v_lib = fs::read(&lib_path).map_err(|e| format!("Verify read Library failed: {}", e))?;
    let v_entries = parse_httpcache_library(&v_lib)?;
    for e in &v_entries {
        if e.hash.is_some() {
            let raw = fs::read(hc_dir.join(&e.hex_name))
                .map_err(|err| format!("Verify read HC/{} failed: {}", e.hex_name, err))?;
            let he = find_http_header_end(&raw).ok_or("Verify header end missing")?;
            let actual = raw.len() - (he + 4);
            if actual != e.body_len {
                return Err(format!(
                    "News cache verification mismatch on HC/{}: {} != {}",
                    e.hex_name, actual, e.body_len
                ));
            }
        }
    }

    on_progress(
        patched,
        matched_total,
        &format!("News translated & verified ({}/{} in {})!", patched, matched_total, lang.short_code()),
    );
    log(
        &format!(
            "=== COMPLETE [News {}]! Patched & verified: {}/{} cached announcements ===",
            lang.label(),
            patched,
            matched_total
        ),
        LogLevel::Ok,
    );
    Ok((patched, matched_total))
}

pub fn rollback_news_cache<L>(cache_base: &Path, log: &mut L) -> Result<usize, String>
where
    L: FnMut(&str, LogLevel),
{
    let bak_dir = cache_base.join("_NewsPatchBackup");
    let bak_lib = bak_dir.join("Library");
    let bak_hc = bak_dir.join("HTTPCache");
    if !bak_lib.is_file() {
        return Ok(0);
    }
    let lib_path = cache_base.join("Library");
    let hc_dir = cache_base.join("HTTPCache");
    let orig_lib = fs::read(&bak_lib).map_err(|e| format!("Read news Library backup failed: {}", e))?;
    let entries = parse_httpcache_library(&orig_lib)?;
    let mut restored = 0usize;
    for e in &entries {
        let src = bak_hc.join(&e.hex_name);
        if src.is_file() {
            fs::copy(&src, hc_dir.join(&e.hex_name))
                .map_err(|err| format!("Restore HC/{} failed: {}", e.hex_name, err))?;
            restored += 1;
        }
    }
    fs::write(&lib_path, &orig_lib).map_err(|e| format!("Restore Library failed: {}", e))?;
    log(
        &format!("  Restored news Library + {} original HTML announcements.", restored),
        LogLevel::Info,
    );
    Ok(restored)
}
