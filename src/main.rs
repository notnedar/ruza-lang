#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod adb;
mod crypto_lz4;
mod patch_engine;
mod updater;

use patch_engine::{LogLevel, PatchLang, ProcessInfo, TargetStatus};
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::platform::windows::WindowExtWindows;
use tao::window::WindowBuilder;
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE};
use wry::{WebContext, WebViewBuilder};

const INDEX_HTML: &str = include_str!("ui/index.html");
const HERO_WEBP: &[u8] = include_bytes!("ui/hero.webp");

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(((input.len() + 2) / 3) * 4);
    for chunk in input.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(TABLE[((n >> 18) & 63) as usize] as char);
        out.push(TABLE[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 { TABLE[((n >> 6) & 63) as usize] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[(n & 63) as usize] as char } else { '=' });
    }
    out
}

#[derive(Deserialize)]
struct IpcCommand {
    cmd: String,
    path: Option<String>,
    lang: Option<String>,
    url: Option<String>,
    update: Option<updater::UpdateInfo>,
    serial: Option<String>,
    adb_path: Option<String>,
    endpoint: Option<String>,
    custom_pack_path: Option<String>,
    font_mode: Option<String>,
}

enum UserEvent {
    EvalJs(String),
    DragWindow,
    MinimizeWindow,
    CloseWindow,
}

fn push_event(proxy: &tao::event_loop::EventLoopProxy<UserEvent>, val: serde_json::Value) {
    let script = format!("window.__onPatcherEvent && window.__onPatcherEvent({});", val);
    let _ = proxy.send_event(UserEvent::EvalJs(script));
}

fn push_log(proxy: &tao::event_loop::EventLoopProxy<UserEvent>, lvl: LogLevel, msg: &str) {
    push_event(proxy, json!({ "type": "log", "level": lvl.as_str(), "message": msg }));
}

fn run_selftest(mock_dir: &str) -> i32 {
    let dir = PathBuf::from(mock_dir);
    let log_path = dir.join("selftest.log");
    let mut lines: Vec<String> = Vec::new();
    let res = (|| -> Result<(), String> {
        lines.push(format!("[INFO] End-to-end bilingual self-test in: {}", mock_dir));

        // 1. Verify initial Original JP
        let s0 = patch_engine::scan_game_folder(&dir, |m, l| {
            lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
        });
        if !s0.iter().all(|x| x.state == "original") {
            return Err("Initial scan did not see all 4 targets as original".into());
        }

        // 2. Patch EN from Original JP
        let mut last_cur = 0usize;
        let rew_en = patch_engine::apply_patch_for_selftest(
            &dir,
            PatchLang::En,
            None,
            |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
            |cur, _tot, _st| last_cur = cur,
        )?;
        lines.push(format!("[OK] EN Rewritten={} (lastProgress={})", rew_en, last_cur));
        let s1 = patch_engine::scan_game_folder(&dir, |m, l| {
            lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
        });
        for t in &s1 {
            lines.push(format!("[EN_SHA] {} = {}", t.id, t.sha256));
        }
        if !s1.iter().all(|x| x.state == "patched_en" && x.has_backup) {
            return Err("Post-EN scan did not verify all 4 targets as patched_en + backed up!".into());
        }

        // 3. Switch directly from EN -> ID
        let rew_id = patch_engine::apply_patch_for_selftest(
            &dir,
            PatchLang::Id,
            None,
            |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
            |cur, _tot, _st| last_cur = cur,
        )?;
        lines.push(format!("[OK] ID Rewritten={} (lastProgress={})", rew_id, last_cur));
        let s2 = patch_engine::scan_game_folder(&dir, |m, l| {
            lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
        });
        for t in &s2 {
            lines.push(format!("[ID_SHA] {} = {}", t.id, t.sha256));
        }
        if !s2.iter().all(|x| x.state == "patched_id" && x.has_backup) {
            return Err("Post-ID scan did not verify all 4 targets as patched_id + backed up!".into());
        }

        // 4. Switch directly from ID -> EN
        let _ = patch_engine::apply_patch_for_selftest(
            &dir,
            PatchLang::En,
            None,
            |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
            |_, _, _| {},
        )?;
        let s3 = patch_engine::scan_game_folder(&dir, |_, _| {});
        if !s3.iter().all(|x| x.state == "patched_en" && x.has_backup) {
            return Err("Switch ID -> EN did not verify all 4 targets as patched_en!".into());
        }

        // 5. Rollback to Original JP
        let restored = patch_engine::rollback_to_original_for_selftest(
            &dir,
            None,
            |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
            |_, _, _| {},
        )?;
        let s4 = patch_engine::scan_game_folder(&dir, |m, l| {
            lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
        });
        if restored != 4 || !s4.iter().all(|x| x.state == "original") {
            return Err("Rollback did not restore all 4 targets to exact original SHA-256!".into());
        }
        // 6. Optional News cache self-test if _MockNewsCache/Library is present
        let mock_news = dir.join("_MockNewsCache");
        if mock_news.join("Library").is_file() {
            let orig_lib_sha = crypto_lz4::sha256_file(&mock_news.join("Library"))?;
            let (en_n, en_tot) = patch_engine::patch_news_cache_for_selftest(
                &mock_news,
                PatchLang::En,
                |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
                |_, _, _| {},
            )?;
            let (id_n, id_tot) = patch_engine::patch_news_cache_for_selftest(
                &mock_news,
                PatchLang::Id,
                |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
                |_, _, _| {},
            )?;
            if en_n == 0 || en_n != en_tot || id_n != id_tot {
                return Err(format!("News self-test count mismatch: EN={}/{}, ID={}/{}", en_n, en_tot, id_n, id_tot));
            }
            let rb_n = patch_engine::rollback_news_cache(&mock_news, &mut |m, l| {
                lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
            })?;
            let post_lib_sha = crypto_lz4::sha256_file(&mock_news.join("Library"))?;
            if rb_n != en_n || !post_lib_sha.eq_ignore_ascii_case(&orig_lib_sha) {
                return Err("News rollback did not restore exact original Library SHA-256!".into());
            }
            lines.push(format!("[OK] NEWS SELFTEST PASSED 100%! ({} announcements EN -> ID -> Rollback JP)", en_n));
        }

        // 7. Client version sync detector self-test (synced vs mismatch)
        let _ = std::fs::write(
            dir.join("game-launcher-config.json"),
            r#"{"version":"1.0.2"}"#,
        );
        let ver_ok = patch_engine::detect_client_version(&dir, &s4);
        if ver_ok.status != "synced" || ver_ok.detected_version != "1.0.2" {
            return Err(format!("Version sync check failed on 1.0.2: status={}", ver_ok.status));
        }
        let _ = std::fs::write(
            dir.join("game-launcher-config.json"),
            r#"{"version":"1.0.3"}"#,
        );
        let ver_bad = patch_engine::detect_client_version(&dir, &s4);
        if ver_bad.status != "mismatch" || ver_bad.reasons.is_empty() {
            return Err("Version sync check failed to detect 1.0.3 mismatch!".into());
        }
        let _ = std::fs::remove_file(dir.join("game-launcher-config.json"));
        lines.push("[OK] VERSION SYNC SELFTEST PASSED 100%! (1.0.2 synced -> 1.0.3 mismatch detected)".into());

        // 8. Auto-updater logic & URL allowlist self-test
        if !updater::is_newer_version("v0.5.1", "0.5.0")
            || updater::is_newer_version("v0.5.0", "0.5.0")
            || updater::is_newer_version("v0.4.3", "0.5.0")
        {
            return Err("SemVer comparison self-test failed!".into());
        }
        let sample_sha = "8B4B618001948E05EA6E9803A8A56E9B806CA59A40489BACB5414409500CB013  RuzaLangPatcher.exe\n";
        if updater::parse_sha256sums(sample_sha, "RuzaLangPatcher.exe").as_deref()
            != Some("8B4B618001948E05EA6E9803A8A56E9B806CA59A40489BACB5414409500CB013")
        {
            return Err("SHA256SUMS parser self-test failed!".into());
        }
        if updater::validate_allowed_https_url("https://github.com/notnedar/ruza-lang/releases/latest").is_err()
            || updater::validate_allowed_https_url("http://github.com/notnedar/ruza-lang").is_ok()
            || updater::validate_allowed_https_url("https://evil.example.com/payload.exe").is_ok()
        {
            return Err("Update URL allowlist self-test failed!".into());
        }
        lines.push("[OK] AUTO-UPDATER SELFTEST PASSED 100%! (SemVer + SHA256SUMS + HTTPS host allowlist)".into());

        lines.push("[OK] BILINGUAL SELFTEST PASSED 100%! (Original -> EN -> ID -> EN -> Rollback JP)".into());
        Ok(())
    })();

    if let Err(e) = &res {
        lines.push(format!("[ERR] SELFTEST FAILED: {}", e));
    }
    let _ = std::fs::write(&log_path, lines.join("\n"));
    if res.is_ok() { 0 } else { 1 }
}

fn run_selftest_android(mock_dir: &str) -> i32 {
    let dir = PathBuf::from(mock_dir);
    let log_path = dir.join("selftest_android.log");
    let mut lines: Vec<String> = Vec::new();
    let res = (|| -> Result<(), String> {
        lines.push(format!("[INFO] Android bundle bilingual self-test in: {}", mock_dir));
        let vs_path = dir.join("android_vstring.r");
        let ui_path = dir.join("android_static_ui.r");
        if !vs_path.is_file() || !ui_path.is_file() {
            return Err("Missing android_vstring.r or android_static_ui.r in selftest dir".into());
        }
        let vs_orig = std::fs::read(&vs_path).map_err(|e| e.to_string())?;
        let ui_orig = std::fs::read(&ui_path).map_err(|e| e.to_string())?;
        let vs_orig_sha = crypto_lz4::sha256_bytes(&vs_orig);
        let ui_orig_sha = crypto_lz4::sha256_bytes(&ui_orig);
        lines.push(format!("[ORIG_SHA] vstring = {}", vs_orig_sha));
        lines.push(format!("[ORIG_SHA] static_ui = {}", ui_orig_sha));
        let (ui_sub_off, _) = patch_engine::find_static_ui_subbundle_in_bytes(&ui_orig)
            .ok_or("Could not discover Static UI sub-bundle in android_static_ui.r")?;

        for (lang, tag) in [(PatchLang::En, "EN"), (PatchLang::Id, "ID")] {
            let (vs_out, vs_cnt) = patch_engine::patch_android_vstring_bytes(
                &vs_orig,
                lang,
                None,
                &mut |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
                &mut |_, _| {},
            )?;
            let (ui_out, ui_cnt) = patch_engine::patch_static_ui_bytes(
                &ui_orig,
                ui_sub_off,
                lang,
                None,
                &mut |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
                &mut |_, _| {},
            )?;
            if vs_out.len() != vs_orig.len() || ui_out.len() != ui_orig.len() {
                return Err(format!("Android {} output byte size mismatch!", tag));
            }
            if vs_cnt < 74055 || ui_cnt != 3135 {
                return Err(format!("Android {} count mismatch: vs={}, ui={}", tag, vs_cnt, ui_cnt));
            }
            let vs_sha = crypto_lz4::sha256_bytes(&vs_out);
            let ui_sha = crypto_lz4::sha256_bytes(&ui_out);
            lines.push(format!("[AN_{}_SHA] vstring = {} (cnt={})", tag, vs_sha, vs_cnt));
            lines.push(format!("[AN_{}_SHA] static_ui = {} (cnt={})", tag, ui_sha, ui_cnt));
        }
        lines.push("[OK] ANDROID UNIVERSAL BILINGUAL SELFTEST PASSED 100%!".into());
        Ok(())
    })();
    if let Err(e) = &res {
        lines.push(format!("[ERR] ANDROID SELFTEST FAILED: {}", e));
    }
    let _ = std::fs::write(&log_path, lines.join("\n"));
    if res.is_ok() { 0 } else { 1 }
}

fn run_cli_android_scan(out_log: &str) -> i32 {
    let mut lines: Vec<String> = Vec::new();
    let res = (|| -> Result<(), String> {
        let adb_bin = adb::detect_adb_binary(None).ok_or("ADB binary not found")?;
        lines.push(format!("[INFO] ADB: {}", adb_bin.display()));
        let devs = adb::list_devices(&adb_bin)?;
        lines.push(format!("[INFO] Devices found: {}", devs.len()));
        for d in &devs {
            lines.push(format!(
                "  - {} ({}, Android {}, state={}, has_game={})",
                d.serial, d.model, d.android_ver, d.state, d.has_game
            ));
        }
        // Scan every ready device so CLI testing shows both installed & uninstalled devices cleanly
        for dev in devs.iter().filter(|d| d.state == "device") {
            let scan = adb::scan_android_device(&adb_bin, &dev.serial, |m, l| {
                lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
            })?;
            lines.push(format!(
                "[OK] Device={} Installed={} RunningPIDs={:?} Ver={} (r{}) Status={}",
                dev.serial,
                scan.installed,
                scan.running_pids,
                scan.version.detected_version,
                scan.version.detected_res_rev,
                scan.version.status
            ));
        }
        Ok(())
    })();
    if let Err(e) = &res {
        lines.push(format!("[ERR] {}", e));
    }
    let _ = std::fs::write(out_log, lines.join("\n"));
    if res.is_ok() { 0 } else { 1 }
}

fn run_cli_android_patch(mode: &str, out_log: &str) -> i32 {
    let mut lines: Vec<String> = Vec::new();
    let res = (|| -> Result<(), String> {
        let adb_bin = adb::detect_adb_binary(None).ok_or("ADB binary not found")?;
        let devs = adb::list_devices(&adb_bin)?;
        let dev = devs
            .iter()
            .find(|d| d.state == "device" && d.has_game)
            .or_else(|| devs.iter().find(|d| d.state == "device"))
            .ok_or("No ready Android device found")?;
        lines.push(format!("[INFO] Target Android device: {} ({})", dev.serial, dev.model));
        if mode.eq_ignore_ascii_case("rollback") {
            let n = adb::rollback_android_device(
                &adb_bin,
                &dev.serial,
                |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
                |_, _, _| {},
            )?;
            lines.push(format!("[OK] Rollback restored {} targets", n));
        } else {
            let lang = PatchLang::from_opt(Some(mode));
            let n = adb::apply_android_patch(
                &adb_bin,
                &dev.serial,
                lang,
                None,
                |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
                |_, _, _| {},
            )?;
            lines.push(format!("[OK] Patched {} strings ({})", n, lang.short_code()));
        }
        let scan = adb::scan_android_device(&adb_bin, &dev.serial, |m, l| {
            lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
        })?;
        for t in &scan.targets {
            lines.push(format!("[POST] {} state={} bak={} sha={}", t.id, t.state, t.has_backup, t.sha256));
        }
        Ok(())
    })();
    if let Err(e) = &res {
        lines.push(format!("[ERR] {}", e));
    }
    let _ = std::fs::write(out_log, lines.join("\n"));
    if res.is_ok() { 0 } else { 1 }
}

fn run_cli_scan(game_dir: &str, out_log: &str) -> i32 {
    let dir = PathBuf::from(game_dir);
    let mut lines: Vec<String> = Vec::new();
    let procs = patch_engine::find_running_game_processes();
    lines.push(format!(
        "[INFO] Running processes: {}",
        if procs.is_empty() {
            "NONE".to_string()
        } else {
            procs.iter().map(|p| format!("{}({})", p.name, p.pid)).collect::<Vec<_>>().join(", ")
        }
    ));
    let targets = patch_engine::scan_game_folder(&dir, |m, l| {
        lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
    });
    let active_sum: usize = targets
        .iter()
        .filter(|t| t.state == "patched_en" || t.state == "patched_id")
        .map(|t| t.string_count)
        .sum();
    let bak_cnt = targets.iter().filter(|t| t.has_backup).count();
    lines.push(format!(
        "[OK] Summary: {}/{} strings patched, backups={}/{}",
        active_sum,
        patch_engine::TOTAL_STRINGS,
        bak_cnt,
        targets.len()
    ));
    let _ = std::fs::write(out_log, lines.join("\n"));
    let all_patched = targets.iter().all(|t| (t.state == "patched_en" || t.state == "patched_id") && t.has_backup);
    if all_patched { 0 } else { 2 }
}

fn enable_frameless_dwm_corners(hwnd: isize) {
    // DWMWCP_ROUND = 2 (Windows 11 native rounded corners + subtle drop shadow on frameless windows)
    let pref: i32 = 2;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd as _,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            &pref as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

fn webview_data_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join("RuzaLang").join("WebView2");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    updater::schedule_old_exe_cleanup(&args);
    if args.len() >= 3 && args[1].eq_ignore_ascii_case("--selftest") {
        std::process::exit(run_selftest(&args[2]));
    }
    if args.len() >= 3 && args[1].eq_ignore_ascii_case("--selftest-android") {
        std::process::exit(run_selftest_android(&args[2]));
    }
    if args.len() >= 2 && args[1].eq_ignore_ascii_case("--android-scan") {
        let out = args.get(2).map(|s| s.as_str()).unwrap_or("android_scan.log");
        std::process::exit(run_cli_android_scan(out));
    }
    if args.len() >= 3 && args[1].eq_ignore_ascii_case("--android-patch") {
        let out = args.get(3).map(|s| s.as_str()).unwrap_or("android_patch.log");
        std::process::exit(run_cli_android_patch(&args[2], out));
    }
    if args.len() >= 3 && args[1].eq_ignore_ascii_case("--scan") {
        let out = args.get(3).map(|s| s.as_str()).unwrap_or("scan.log");
        std::process::exit(run_cli_scan(&args[2], out));
    }
    if args.len() >= 2 && args[1].eq_ignore_ascii_case("--check-update") {
        let out_file = args.get(2).map(|s| s.as_str());
        match updater::check_for_update() {
            Ok(info) => {
                let txt = serde_json::to_string_pretty(&info).unwrap_or_default();
                if let Some(f) = out_file {
                    let _ = std::fs::write(f, &txt);
                }
                std::process::exit(0);
            }
            Err(e) => {
                if let Some(f) = out_file {
                    let _ = std::fs::write(f, format!("ERR: {}", e));
                }
                std::process::exit(1);
            }
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("Ruza-Lang Patcher v0.9.5 — EN / ID + Custom Packs")
        .with_inner_size(LogicalSize::new(960.0, 580.0))
        .with_decorations(false)
        .with_resizable(false)
        .build(&event_loop)
        .expect("Failed to create window");

    enable_frameless_dwm_corners(window.hwnd());

    // Inline the embedded WebP hero art as a data URI for standalone single-exe execution
    let hero_uri = format!("url('data:image/webp;base64,{}')", base64_encode(HERO_WEBP));
    let html_with_art = INDEX_HTML.replace("url('hero.webp')", &hero_uri);

    // Background process guard poller (every 2 seconds)
    let poll_proxy = proxy.clone();
    thread::spawn(move || loop {
        let procs: Vec<ProcessInfo> = patch_engine::find_running_game_processes();
        push_event(&poll_proxy, json!({ "type": "processes", "processes": procs }));
        thread::sleep(Duration::from_secs(2));
    });

    // Store WebView2 profile in %LOCALAPPDATA%\RuzaLang\WebView2 so no *.exe.WebView2 folder is dropped next to the executable
    let mut web_context = WebContext::new(Some(webview_data_dir()));
    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::with_web_context(&mut web_context)
        .with_html(html_with_art)
        .with_background_color((20, 27, 38, 255))
        .with_ipc_handler(move |req| {
            let body = req.body().to_string();
            let parsed: IpcCommand = match serde_json::from_str(&body) {
                Ok(c) => c,
                Err(_) => return,
            };
            let p = ipc_proxy.clone();
            let lang = PatchLang::from_opt(parsed.lang.as_deref());
            match parsed.cmd.as_str() {
                "win_drag" => {
                    let _ = p.send_event(UserEvent::DragWindow);
                }
                "win_minimize" => {
                    let _ = p.send_event(UserEvent::MinimizeWindow);
                }
                "win_close" => {
                    let _ = p.send_event(UserEvent::CloseWindow);
                }
                "init" | "auto_detect" => {
                    let is_init = parsed.cmd == "init";
                    if is_init {
                        let up_proxy = p.clone();
                        thread::spawn(move || {
                            if let Ok(info) = updater::check_for_update() {
                                if info.has_update {
                                    push_log(
                                        &up_proxy,
                                        LogLevel::Info,
                                        &format!(
                                            "New patcher release available: {} (current: {}).",
                                            info.latest_version, info.current_version
                                        ),
                                    );
                                }
                                push_event(
                                    &up_proxy,
                                    json!({ "type": "update_status", "silent": true, "info": info }),
                                );
                            }
                        });
                    }
                    thread::spawn(move || {
                        if is_init {
                            push_log(&p, LogLevel::Info, "Ruza-Lang Translation Patcher v0.9.5 (Hotfix-Proof Universal Engine · EN / ID + Custom Packs · PC + Android ADB) initialized.");
                            push_log(&p, LogLevel::Info, "Safety: Dynamic bundle auto-discovery, exact byte sizes & SHA-256 verification.");
                        }
                        let procs = patch_engine::find_running_game_processes();
                        push_event(&p, json!({ "type": "processes", "processes": procs }));
                        let detected = patch_engine::auto_detect_game_folder();
                        if !detected.is_empty() {
                            push_event(&p, json!({ "type": "folder", "path": detected, "valid": true }));
                            push_log(&p, LogLevel::Ok, &format!("Detected target folder: {}", detected));
                            do_scan(&p, &detected);
                        } else {
                            push_event(&p, json!({ "type": "folder", "path": "", "valid": false }));
                            push_log(&p, LogLevel::Warn, "Could not auto-detect target folder. Click 'Browse...' to pick it.");
                        }
                    });
                }
                "browse" => {
                    let start = parsed.path.unwrap_or_default();
                    thread::spawn(move || {
                        let mut dlg = rfd::FileDialog::new()
                            .set_title("Select Target Installation Directory");
                        if !start.is_empty() && PathBuf::from(&start).is_dir() {
                            dlg = dlg.set_directory(&start);
                        }
                        if let Some(folder) = dlg.pick_folder() {
                            let s = folder.to_string_lossy().to_string();
                            let valid = patch_engine::is_valid_game_folder(&folder);
                            push_event(&p, json!({ "type": "folder", "path": s, "valid": valid }));
                            if valid {
                                do_scan(&p, &s);
                            } else {
                                push_log(&p, LogLevel::Error, &format!("Invalid target folder: {}", s));
                            }
                        }
                    });
                }
                "scan" => {
                    let dir = parsed.path.unwrap_or_default();
                    thread::spawn(move || do_scan(&p, &dir));
                }
                "pick_custom_pack" => {
                    thread::spawn(move || {
                        let dlg = rfd::FileDialog::new()
                            .set_title("Select Custom Language Pack (.json)")
                            .add_filter("Language Pack JSON", &["json"]);
                        if let Some(file) = dlg.pick_file() {
                            match patch_engine::load_custom_pack(&file) {
                                Ok(pack) => {
                                    push_log(
                                        &p,
                                        LogLevel::Ok,
                                        &format!(
                                            "Loaded Custom Language Pack: {} ({}) · base={} · {} overrides ({})",
                                            pack.meta.name,
                                            pack.meta.code,
                                            pack.meta.base.to_uppercase(),
                                            pack.meta.total_count,
                                            pack.meta.file_name
                                        ),
                                    );
                                    push_event(&p, json!({ "type": "custom_pack", "pack": pack.meta }));
                                }
                                Err(e) => {
                                    push_log(&p, LogLevel::Error, &format!("Custom pack load failed: {}", e));
                                    let title = match lang {
                                        PatchLang::Id => "Paket Bahasa Kustom Tidak Valid",
                                        PatchLang::En => "Invalid Custom Language Pack",
                                    };
                                    push_event(&p, json!({ "type": "alert", "title": title, "body": e }));
                                }
                            }
                        }
                    });
                }
                "clear_custom_pack" => {
                    push_log(&p, LogLevel::Info, "Cleared Custom Language Pack — using built-in EN / ID.");
                    push_event(&p, json!({ "type": "custom_pack", "pack": serde_json::Value::Null }));
                }
                "export_pack_template" => {
                    thread::spawn(move || {
                        let dlg = rfd::FileDialog::new()
                            .set_title("Save Custom Language Pack Template")
                            .set_file_name("ruza-custom-pack-template.json")
                            .add_filter("Language Pack JSON", &["json"]);
                        if let Some(dst) = dlg.save_file() {
                            match patch_engine::export_custom_pack_template(&dst) {
                                Ok(()) => {
                                    let s = dst.to_string_lossy().to_string();
                                    push_log(&p, LogLevel::Ok, &format!("Saved Custom Language Pack template: {}", s));
                                    let (title, body) = match lang {
                                        PatchLang::Id => (
                                            "Template Tersimpan!",
                                            format!("Template JSON berhasil disimpan di:\n{}\n\nEdit file tersebut, lalu klik Muat JSON untuk langsung memasang bahasamu sendiri di PC maupun Android.", s),
                                        ),
                                        PatchLang::En => (
                                            "Template Saved!",
                                            format!("Starter JSON template saved to:\n{}\n\nEdit the file and click Load JSON to install your own language pack on both PC and Android.", s),
                                        ),
                                    };
                                    push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                                }
                                Err(e) => {
                                    push_log(&p, LogLevel::Error, &e);
                                    push_event(&p, json!({ "type": "alert", "title": "Export Failed", "body": e }));
                                }
                            }
                        }
                    });
                }
                "patch" => {
                    let dir = parsed.path.unwrap_or_default();
                    let custom_path = parsed.custom_pack_path;
                    let font_mode = patch_engine::FontMode::from_opt(parsed.font_mode.as_deref());
                    thread::spawn(move || {
                        let path = PathBuf::from(&dir);
                        if !patch_engine::is_valid_game_folder(&path) {
                            push_log(&p, LogLevel::Error, "Invalid target folder! Select a valid directory first.");
                            let (title, msg) = match lang {
                                PatchLang::Id => (
                                    "Folder Tidak Valid",
                                    "Pilih folder instalasi target yang valid terlebih dahulu.",
                                ),
                                PatchLang::En => (
                                    "Invalid Folder",
                                    "Please select a valid target installation directory first.",
                                ),
                            };
                            push_event(&p, json!({ "type": "alert", "title": title, "body": msg }));
                            return;
                        }
                        let procs = patch_engine::find_running_game_processes();
                        push_event(&p, json!({ "type": "processes", "processes": procs }));
                        if !procs.is_empty() {
                            let list = procs.iter().map(|x| format!("• {} (PID {})", x.name, x.pid)).collect::<Vec<_>>().join("\n");
                            push_log(&p, LogLevel::Error, "BLOCKED: Game or launcher is running!");
                            let (title, body) = match lang {
                                PatchLang::Id => (
                                    "Tutup Game / Launcher Dulu",
                                    format!("Harap tutup dulu proses berikut sebelum install patch:\n{}", list),
                                ),
                                PatchLang::En => (
                                    "Close Game / Launcher First",
                                    format!("Please close these processes before patching:\n{}", list),
                                ),
                            };
                            push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            return;
                        }
                        let custom_pack = match custom_path.as_deref().filter(|s| !s.trim().is_empty()) {
                            Some(cp) => match patch_engine::load_custom_pack(std::path::Path::new(cp)) {
                                Ok(pk) => {
                                    push_event(&p, json!({ "type": "custom_pack", "pack": pk.meta }));
                                    Some(pk)
                                }
                                Err(e) => {
                                    push_log(&p, LogLevel::Error, &format!("Custom pack load failed: {}", e));
                                    push_event(&p, json!({ "type": "alert", "title": "Custom Pack Error", "body": e }));
                                    return;
                                }
                            },
                            None => None,
                        };
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        match patch_engine::apply_patch(
                            &path,
                            lang,
                            custom_pack.as_ref(),
                            font_mode,
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok(n) => {
                                let (title, body) = if let Some(ref c) = custom_pack {
                                    match lang {
                                        PatchLang::Id => (
                                            "Patch Bahasa Kustom Selesai!".to_string(),
                                            format!(
                                                "Paket kustom {} ({}) berhasil dipasang & diverifikasi 100%!\nBase: {} · Override: {} · Total di-rewrite: {} strings.",
                                                c.meta.name, c.meta.code, c.meta.base.to_uppercase(), c.meta.total_count, n
                                            ),
                                        ),
                                        PatchLang::En => (
                                            "Custom Language Patch Complete!".to_string(),
                                            format!(
                                                "Custom pack {} ({}) installed & verified 100%!\nBase: {} · Overrides: {} · Total rewritten: {} strings.",
                                                c.meta.name, c.meta.code, c.meta.base.to_uppercase(), c.meta.total_count, n
                                            ),
                                        ),
                                    }
                                } else {
                                    match lang {
                                        PatchLang::Id => (
                                            "Patch Bahasa Indonesia Selesai!".to_string(),
                                            format!("Patch Bahasa Indonesia (ID) berhasil dipasang & diverifikasi 100%!\nTotal string di-rewrite: {} strings.", n),
                                        ),
                                        PatchLang::En => (
                                            "English Patch Complete!".to_string(),
                                            format!("English (EN) patch installed & verified 100%!\nTotal strings rewritten: {} strings.", n),
                                        ),
                                    }
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("PATCH FAILED: {}", e));
                                push_event(&p, json!({ "type": "alert", "title": "Patch Failed", "body": e }));
                            }
                        }
                        do_scan(&p, &dir);
                    });
                }
                "patch_news" => {
                    thread::spawn(move || {
                        let procs = patch_engine::find_running_game_processes();
                        push_event(&p, json!({ "type": "processes", "processes": procs }));
                        if !procs.is_empty() {
                            let list = procs.iter().map(|x| format!("• {} (PID {})", x.name, x.pid)).collect::<Vec<_>>().join("\n");
                            push_log(&p, LogLevel::Error, "BLOCKED: Target process is running!");
                            let (title, body) = match lang {
                                PatchLang::Id => (
                                    "Tutup Proses Target Dulu",
                                    format!("Harap tutup dulu proses berikut sebelum translate news:\n{}", list),
                                ),
                                PatchLang::En => (
                                    "Close Running Process First",
                                    format!("Please close these processes before translating news:\n{}", list),
                                ),
                            };
                            push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            return;
                        }
                        let cache_dir = match patch_engine::default_news_cache_dir() {
                            Ok(d) => d,
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &e);
                                push_event(&p, json!({ "type": "alert", "title": "News Cache Error", "body": e }));
                                return;
                            }
                        };
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        match patch_engine::patch_news_cache(
                            &cache_dir,
                            lang,
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok((patched, total)) => {
                                let (title, body) = match lang {
                                    PatchLang::Id => (
                                        "News Bahasa Indonesia Selesai!",
                                        format!("Berhasil menerjemahkan {}/{} pengumuman cache in-game ke Bahasa Indonesia (ID).\nBackup asli disimpan di _NewsPatchBackup\\.", patched, total),
                                    ),
                                    PatchLang::En => (
                                        "News Translation Complete!",
                                        format!("Successfully translated {}/{} cached in-game announcements to English (EN).\nOriginal backup saved in _NewsPatchBackup\\.", patched, total),
                                    ),
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("NEWS PATCH FAILED: {}", e));
                                push_event(&p, json!({ "type": "alert", "title": "Translate News Failed", "body": e }));
                            }
                        }
                        push_event(&p, json!({ "type": "busy", "value": false }));
                    });
                }
                "rollback" => {
                    let dir = parsed.path.unwrap_or_default();
                    thread::spawn(move || {
                        let path = PathBuf::from(&dir);
                        if !patch_engine::is_valid_game_folder(&path) {
                            push_log(&p, LogLevel::Error, "Invalid target folder! Select a valid directory first.");
                            return;
                        }
                        let procs = patch_engine::find_running_game_processes();
                        push_event(&p, json!({ "type": "processes", "processes": procs }));
                        if !procs.is_empty() {
                            let list = procs.iter().map(|x| format!("• {} (PID {})", x.name, x.pid)).collect::<Vec<_>>().join("\n");
                            push_log(&p, LogLevel::Error, "BLOCKED: Game or launcher is running!");
                            let (title, body) = match lang {
                                PatchLang::Id => (
                                    "Tutup Game / Launcher Dulu",
                                    format!("Harap tutup dulu proses berikut sebelum rollback:\n{}", list),
                                ),
                                PatchLang::En => (
                                    "Close Game / Launcher First",
                                    format!("Please close these processes before rollback:\n{}", list),
                                ),
                            };
                            push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            return;
                        }
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        let news_dir = patch_engine::default_news_cache_dir().ok();
                        match patch_engine::rollback_to_original(
                            &path,
                            news_dir.as_deref(),
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok(n) => {
                                let (title, body) = match lang {
                                    PatchLang::Id => (
                                        "Rollback Selesai",
                                        format!("Berhasil mengembalikan {}/4 file ke Original JP & hash SHA-256 terverifikasi.", n),
                                    ),
                                    PatchLang::En => (
                                        "Rollback Complete",
                                        format!("Successfully restored {}/4 files to Original JP with verified SHA-256 hashes.", n),
                                    ),
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("ROLLBACK FAILED: {}", e));
                                push_event(&p, json!({ "type": "alert", "title": "Rollback Failed", "body": e }));
                            }
                        }
                        do_scan(&p, &dir);
                    });
                }
                "check_update" => {
                    thread::spawn(move || {
                        push_event(&p, json!({ "type": "update_checking", "value": true }));
                        push_log(&p, LogLevel::Info, "Checking GitHub Releases for Ruza-Lang updates...");
                        match updater::check_for_update() {
                            Ok(info) => {
                                if info.has_update {
                                    push_log(
                                        &p,
                                        LogLevel::Ok,
                                        &format!(
                                            "Update available: {} (current: {}).",
                                            info.latest_version, info.current_version
                                        ),
                                    );
                                } else {
                                    push_log(
                                        &p,
                                        LogLevel::Ok,
                                        &format!(
                                            "You are on the latest version ({}).",
                                            info.current_version
                                        ),
                                    );
                                }
                                push_event(
                                    &p,
                                    json!({ "type": "update_status", "silent": false, "info": info }),
                                );
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Warn, &format!("Update check failed: {}", e));
                                push_event(
                                    &p,
                                    json!({ "type": "update_error", "message": e }),
                                );
                            }
                        }
                        push_event(&p, json!({ "type": "update_checking", "value": false }));
                    });
                }
                "install_update" => {
                    let info_opt = parsed.update;
                    thread::spawn(move || {
                        let info = match info_opt {
                            Some(i) => i,
                            None => match updater::check_for_update() {
                                Ok(i) => i,
                                Err(e) => {
                                    push_log(&p, LogLevel::Error, &format!("UPDATE FAILED: {}", e));
                                    push_event(&p, json!({ "type": "alert", "title": "Update Failed", "body": e }));
                                    return;
                                }
                            },
                        };
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        match updater::download_and_apply_update(
                            &info,
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok(_) => {
                                thread::sleep(Duration::from_millis(350));
                                let _ = p.send_event(UserEvent::CloseWindow);
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("UPDATE FAILED: {}", e));
                                push_event(&p, json!({ "type": "busy", "value": false }));
                                let title = match lang {
                                    PatchLang::Id => "Auto-Update Gagal",
                                    PatchLang::En => "Auto-Update Failed",
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": e }));
                            }
                        }
                    });
                }
                "open_url" => {
                    if let Some(u) = parsed.url {
                        let _ = updater::open_safe_release_url(&u);
                    }
                }
                "android_detect" => {
                    let custom_adb = parsed.adb_path;
                    let preferred_serial = parsed.serial;
                    thread::spawn(move || {
                        do_android_detect(&p, custom_adb.as_deref(), preferred_serial.as_deref());
                    });
                }
                "android_connect_ip" => {
                    let custom_adb = parsed.adb_path;
                    let endpoint = parsed.endpoint.unwrap_or_default();
                    thread::spawn(move || {
                        let Some(adb_bin) = adb::detect_adb_binary(custom_adb.as_deref()) else {
                            let (title, body) = match lang {
                                PatchLang::Id => (
                                    "ADB Belum Ditemukan",
                                    "File adb.exe / HD-Adb.exe tidak ditemukan. Unduh Android Platform-Tools atau klik tombol ADB untuk memilih file-nya.",
                                ),
                                PatchLang::En => (
                                    "ADB Not Found",
                                    "Could not locate adb.exe / HD-Adb.exe. Download Android Platform-Tools or click the ADB button to select it.",
                                ),
                            };
                            push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            return;
                        };
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        push_log(&p, LogLevel::Info, &format!("Connecting ADB TCP endpoint {}...", endpoint));
                        match adb::connect_tcp_endpoint(&adb_bin, &endpoint) {
                            Ok(msg) => {
                                push_log(&p, LogLevel::Ok, &format!("ADB connect: {}", msg));
                                do_android_detect(&p, Some(&adb_bin.to_string_lossy()), Some(endpoint.trim()));
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("ADB connect failed: {}", e));
                                push_event(&p, json!({ "type": "busy", "value": false }));
                                let title = match lang {
                                    PatchLang::Id => "Gagal Konek ADB IP",
                                    PatchLang::En => "ADB IP Connect Failed",
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": e }));
                            }
                        }
                    });
                }
                "android_browse_adb" => {
                    let cur_adb = parsed.adb_path.unwrap_or_default();
                    thread::spawn(move || {
                        let mut dlg = rfd::FileDialog::new()
                            .set_title("Select ADB Executable (adb.exe / HD-Adb.exe)")
                            .add_filter("ADB Executable", &["exe"]);
                        if !cur_adb.is_empty() {
                            if let Some(parent) = PathBuf::from(&cur_adb).parent() {
                                if parent.is_dir() {
                                    dlg = dlg.set_directory(parent);
                                }
                            }
                        }
                        if let Some(file) = dlg.pick_file() {
                            let s = file.to_string_lossy().to_string();
                            push_log(&p, LogLevel::Info, &format!("Selected ADB binary: {}", s));
                            do_android_detect(&p, Some(&s), None);
                        }
                    });
                }
                "android_scan" => {
                    let custom_adb = parsed.adb_path;
                    let serial = parsed.serial.unwrap_or_default();
                    thread::spawn(move || {
                        let Some(adb_bin) = adb::detect_adb_binary(custom_adb.as_deref()) else {
                            push_log(&p, LogLevel::Warn, "ADB executable not found. Install Android Platform-Tools or click 'ADB' to select adb.exe / HD-Adb.exe.");
                            push_event(&p, json!({ "type": "android_devices", "adb_path": "", "devices": [] }));
                            return;
                        };
                        if serial.is_empty() {
                            do_android_detect(&p, Some(&adb_bin.to_string_lossy()), None);
                        } else {
                            do_android_scan(&p, &adb_bin, &serial);
                        }
                    });
                }
                "android_patch" => {
                    let custom_adb = parsed.adb_path;
                    let serial = parsed.serial.unwrap_or_default();
                    let custom_path = parsed.custom_pack_path;
                    thread::spawn(move || {
                        let Some(adb_bin) = adb::detect_adb_binary(custom_adb.as_deref()) else {
                            push_event(&p, json!({ "type": "alert", "title": "ADB Not Found", "body": "Could not locate adb.exe or HD-Adb.exe. Click 'ADB' to pick it manually." }));
                            return;
                        };
                        if serial.is_empty() {
                            push_event(&p, json!({ "type": "alert", "title": "No Android Device", "body": "Connect an Android device with USB Debugging or start BlueStacks, then click Scan." }));
                            return;
                        }
                        let custom_pack = match custom_path.as_deref().filter(|s| !s.trim().is_empty()) {
                            Some(cp) => match patch_engine::load_custom_pack(std::path::Path::new(cp)) {
                                Ok(pk) => {
                                    push_event(&p, json!({ "type": "custom_pack", "pack": pk.meta }));
                                    Some(pk)
                                }
                                Err(e) => {
                                    push_log(&p, LogLevel::Error, &format!("Custom pack load failed: {}", e));
                                    push_event(&p, json!({ "type": "alert", "title": "Custom Pack Error", "body": e }));
                                    return;
                                }
                            },
                            None => None,
                        };
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        match adb::apply_android_patch(
                            &adb_bin,
                            &serial,
                            lang,
                            custom_pack.as_ref(),
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "android_progress", "current": cur, "total": tot, "stage": st }));
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok(n) => {
                                let (title, body) = if let Some(ref c) = custom_pack {
                                    match lang {
                                        PatchLang::Id => (
                                            format!("Patch Android ({}) Selesai!", c.meta.code),
                                            format!(
                                                "Paket kustom {} ({}) berhasil dipasang & diverifikasi 100% di {}!\nBase: {} · Override: {} · Total di-rewrite: {} strings.",
                                                c.meta.name, c.meta.code, serial, c.meta.base.to_uppercase(), c.meta.total_count, n
                                            ),
                                        ),
                                        PatchLang::En => (
                                            format!("Android {} Patch Complete!", c.meta.code),
                                            format!(
                                                "Custom pack {} ({}) installed & verified 100% on {}!\nBase: {} · Overrides: {} · Total rewritten: {} strings.",
                                                c.meta.name, c.meta.code, serial, c.meta.base.to_uppercase(), c.meta.total_count, n
                                            ),
                                        ),
                                    }
                                } else {
                                    match lang {
                                        PatchLang::Id => (
                                            "Patch Android (ID) Selesai!".to_string(),
                                            format!("Patch Bahasa Indonesia (ID) berhasil dipasang & diverifikasi 100% di {}!\nTotal string di-rewrite: {} strings (VStringPool + Static UI).", serial, n),
                                        ),
                                        PatchLang::En => (
                                            "Android English Patch Complete!".to_string(),
                                            format!("English (EN) patch installed & verified 100% on {}!\nTotal strings rewritten: {} strings (VStringPool + Static UI).", serial, n),
                                        ),
                                    }
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("ANDROID PATCH FAILED: {}", e));
                                push_event(&p, json!({ "type": "alert", "title": "Android Patch Failed", "body": e }));
                            }
                        }
                        do_android_scan(&p, &adb_bin, &serial);
                    });
                }
                "android_patch_news" => {
                    let custom_adb = parsed.adb_path;
                    let serial = parsed.serial.unwrap_or_default();
                    thread::spawn(move || {
                        let Some(adb_bin) = adb::detect_adb_binary(custom_adb.as_deref()) else {
                            push_event(&p, json!({ "type": "alert", "title": "ADB Not Found", "body": "Could not locate adb.exe or HD-Adb.exe." }));
                            return;
                        };
                        if serial.is_empty() {
                            return;
                        }
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        match adb::patch_android_news(
                            &adb_bin,
                            &serial,
                            lang,
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "android_progress", "current": cur, "total": tot, "stage": st }));
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok((patched, total)) => {
                                let (title, body) = match lang {
                                    PatchLang::Id => (
                                        "News Android (ID) Selesai!",
                                        format!("Berhasil menerjemahkan {}/{} pengumuman cache di Android ({}) ke Bahasa Indonesia (ID).", patched, total, serial),
                                    ),
                                    PatchLang::En => (
                                        "Android News Translation Complete!",
                                        format!("Successfully translated {}/{} cached Android announcements on {} to English (EN).", patched, total, serial),
                                    ),
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("ANDROID NEWS FAILED: {}", e));
                                push_event(&p, json!({ "type": "alert", "title": "Android News Failed", "body": e }));
                            }
                        }
                        do_android_scan(&p, &adb_bin, &serial);
                    });
                }
                "android_rollback" => {
                    let custom_adb = parsed.adb_path;
                    let serial = parsed.serial.unwrap_or_default();
                    thread::spawn(move || {
                        let Some(adb_bin) = adb::detect_adb_binary(custom_adb.as_deref()) else {
                            push_event(&p, json!({ "type": "alert", "title": "ADB Not Found", "body": "Could not locate adb.exe or HD-Adb.exe." }));
                            return;
                        };
                        if serial.is_empty() {
                            return;
                        }
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        match adb::rollback_android_device(
                            &adb_bin,
                            &serial,
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "android_progress", "current": cur, "total": tot, "stage": st }));
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok(n) => {
                                let (title, body) = match lang {
                                    PatchLang::Id => (
                                        "Rollback Android Selesai",
                                        format!("Berhasil mengembalikan {}/2 bundle Android ({}) ke Original JP dengan SHA-256 terverifikasi.", n, serial),
                                    ),
                                    PatchLang::En => (
                                        "Android Rollback Complete",
                                        format!("Successfully restored {}/2 Android bundles on {} to Original JP with verified SHA-256 hashes.", n, serial),
                                    ),
                                };
                                push_event(&p, json!({ "type": "alert", "title": title, "body": body }));
                            }
                            Err(e) => {
                                push_log(&p, LogLevel::Error, &format!("ANDROID ROLLBACK FAILED: {}", e));
                                push_event(&p, json!({ "type": "alert", "title": "Android Rollback Failed", "body": e }));
                            }
                        }
                        do_android_scan(&p, &adb_bin, &serial);
                    });
                }
                _ => {}
            }
        })
        .build(&window)
        .expect("Failed to initialize Edge WebView2");

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => *control_flow = ControlFlow::Exit,
            Event::UserEvent(UserEvent::EvalJs(script)) => {
                let _ = webview.evaluate_script(&script);
            }
            Event::UserEvent(UserEvent::DragWindow) => {
                let _ = window.drag_window();
            }
            Event::UserEvent(UserEvent::MinimizeWindow) => {
                window.set_minimized(true);
            }
            Event::UserEvent(UserEvent::CloseWindow) => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

fn do_scan(proxy: &tao::event_loop::EventLoopProxy<UserEvent>, dir_str: &str) {
    let dir = PathBuf::from(dir_str);
    let valid = patch_engine::is_valid_game_folder(&dir);
    push_event(proxy, json!({ "type": "folder", "path": dir_str, "valid": valid }));
    if !valid {
        push_log(proxy, LogLevel::Error, &format!("Invalid game folder: {}", dir_str));
        push_event(proxy, json!({ "type": "busy", "value": false }));
        return;
    }
    push_event(proxy, json!({ "type": "busy", "value": true }));
    push_log(proxy, LogLevel::Info, &format!("Scanning {}...", dir_str));
    let targets: Vec<TargetStatus> = patch_engine::scan_game_folder(&dir, |m, l| push_log(proxy, l, m));
    let ver = patch_engine::detect_client_version(&dir, &targets);
    let ver_display = match (ver.detected_version.is_empty(), ver.detected_res_rev.is_empty()) {
        (false, false) => format!("v{} (r{})", ver.detected_version, ver.detected_res_rev),
        (false, true) => format!("v{}", ver.detected_version),
        (true, false) => format!("r{}", ver.detected_res_rev),
        (true, true) => format!("v{} (hash-verified)", ver.expected_version),
    };
    if ver.status == "synced" {
        push_log(
            proxy,
            LogLevel::Ok,
            &format!("Client version check: {} — Synced with patcher.", ver_display),
        );
    } else {
        push_log(
            proxy,
            LogLevel::Warn,
            &format!(
                "Client version mismatch ({}) vs verified v{} (r{}): {}. Waiting for a patcher update is recommended (install remains allowed).",
                ver_display,
                ver.expected_version,
                ver.expected_res_rev,
                ver.reasons.join("; ")
            ),
        );
    }
    let en_sum: usize = targets.iter().filter(|t| t.state == "patched_en").map(|t| t.string_count).sum();
    let id_sum: usize = targets.iter().filter(|t| t.state == "patched_id").map(|t| t.string_count).sum();
    let cu_sum: usize = targets.iter().filter(|t| t.state == "patched_custom").map(|t| t.string_count).sum();
    let summary = if cu_sum > 0 && en_sum == 0 && id_sum == 0 {
        format!("Scan done: {} strings currently patched with Custom Language Pack.", cu_sum)
    } else if id_sum > 0 && en_sum == 0 {
        format!("Scan done: {} / {} strings currently in Bahasa Indonesia (ID).", id_sum, patch_engine::TOTAL_STRINGS)
    } else if en_sum > 0 && id_sum == 0 {
        format!("Scan done: {} / {} strings currently in English (EN).", en_sum, patch_engine::TOTAL_STRINGS)
    } else if en_sum > 0 || id_sum > 0 || cu_sum > 0 {
        format!("Scan done: mixed state (EN={}, ID={}, Custom={}).", en_sum, id_sum, cu_sum)
    } else {
        format!("Scan done: Original JP (0 / {} strings translated).", patch_engine::TOTAL_STRINGS)
    };
    let st = patch_engine::load_patch_state(&dir.join("_ENPatchBackup"));
    push_log(proxy, LogLevel::Ok, &summary);
    push_event(proxy, json!({ "type": "scan_result", "targets": targets, "version": ver, "font_mode": st.font_mode }));
}

fn do_android_detect(
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    custom_adb: Option<&str>,
    preferred_serial: Option<&str>,
) {
    push_event(proxy, json!({ "type": "busy", "value": true }));
    let Some(adb_bin) = adb::detect_adb_binary(custom_adb) else {
        push_log(
            proxy,
            LogLevel::Warn,
            "Android ADB binary not found. Install Android Platform-Tools or select adb.exe / HD-Adb.exe.",
        );
        push_event(proxy, json!({ "type": "android_devices", "adb_path": "", "devices": [] }));
        push_event(proxy, json!({ "type": "busy", "value": false }));
        return;
    };
    let adb_str = adb_bin.to_string_lossy().to_string();
    push_log(proxy, LogLevel::Info, &format!("Scanning Android devices via {}...", adb_str));
    match adb::list_devices(&adb_bin) {
        Ok(devices) => {
            push_event(
                proxy,
                json!({ "type": "android_devices", "adb_path": adb_str, "devices": devices }),
            );
            let chosen = preferred_serial
                .and_then(|s| devices.iter().find(|d| d.serial == s && d.state == "device"))
                .or_else(|| devices.iter().find(|d| d.state == "device" && d.has_game))
                .or_else(|| devices.iter().find(|d| d.state == "device"));
            if let Some(dev) = chosen {
                do_android_scan(proxy, &adb_bin, &dev.serial);
            } else {
                if devices.is_empty() {
                    push_log(
                        proxy,
                        LogLevel::Warn,
                        "No Android device detected. Enable USB Debugging or open BlueStacks (ADB enabled), then click Scan.",
                    );
                } else {
                    push_log(
                        proxy,
                        LogLevel::Warn,
                        "Android device found but unauthorized/offline. Accept the USB Debugging prompt on the device.",
                    );
                }
                push_event(proxy, json!({ "type": "busy", "value": false }));
            }
        }
        Err(e) => {
            push_log(proxy, LogLevel::Error, &format!("ADB device scan failed: {}", e));
            push_event(proxy, json!({ "type": "android_devices", "adb_path": adb_str, "devices": [] }));
            push_event(proxy, json!({ "type": "busy", "value": false }));
        }
    }
}

fn do_android_scan(
    proxy: &tao::event_loop::EventLoopProxy<UserEvent>,
    adb_bin: &std::path::Path,
    serial: &str,
) {
    push_event(proxy, json!({ "type": "busy", "value": true }));
    push_log(proxy, LogLevel::Info, &format!("Inspecting Android device {}...", serial));
    match adb::scan_android_device(adb_bin, serial, |m, l| push_log(proxy, l, m)) {
        Ok(res) => {
            if !res.running_pids.is_empty() {
                push_log(
                    proxy,
                    LogLevel::Warn,
                    &format!(
                        "Android client is currently running on {} (PID {:?}) — close it before patching.",
                        serial, res.running_pids
                    ),
                );
            }
            push_event(proxy, json!({ "type": "android_scan_result", "result": res }));
        }
        Err(e) => {
            push_log(proxy, LogLevel::Error, &format!("Android scan failed: {}", e));
        }
    }
    push_event(proxy, json!({ "type": "busy", "value": false }));
}
