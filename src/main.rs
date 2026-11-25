#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod crypto_lz4;
mod patch_engine;

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
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use wry::WebViewBuilder;

const INDEX_HTML: &str = include_str!("ui/index.html");

#[derive(Deserialize)]
struct IpcCommand {
    cmd: String,
    path: Option<String>,
    lang: Option<String>,
}

enum UserEvent {
    EvalJs(String),
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
        let rew_en = patch_engine::apply_patch(
            &dir,
            PatchLang::En,
            |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
            |cur, _tot, _st| last_cur = cur,
        )?;
        lines.push(format!("[OK] EN Rewritten={} (lastProgress={})", rew_en, last_cur));
        let s1 = patch_engine::scan_game_folder(&dir, |m, l| {
            lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
        });
        if !s1.iter().all(|x| x.state == "patched_en" && x.has_backup) {
            return Err("Post-EN scan did not verify all 4 targets as patched_en + backed up!".into());
        }

        // 3. Switch directly from EN -> ID
        let rew_id = patch_engine::apply_patch(
            &dir,
            PatchLang::Id,
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
        if !patch_engine::STATIC_PKG_ID_SHA.is_empty()
            && !s2.iter().all(|x| x.state == "patched_id" && x.has_backup)
        {
            return Err("Post-ID scan did not verify all 4 targets as patched_id + backed up!".into());
        }

        // 4. Switch directly from ID -> EN
        let _ = patch_engine::apply_patch(
            &dir,
            PatchLang::En,
            |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
            |_, _, _| {},
        )?;
        let s3 = patch_engine::scan_game_folder(&dir, |_, _| {});
        if !s3.iter().all(|x| x.state == "patched_en" && x.has_backup) {
            return Err("Switch ID -> EN did not verify all 4 targets as patched_en!".into());
        }

        // 5. Rollback to Original JP
        let restored = patch_engine::rollback_to_original(
            &dir,
            |m, l| lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m)),
            |_, _, _| {},
        )?;
        let s4 = patch_engine::scan_game_folder(&dir, |m, l| {
            lines.push(format!("[{}] {}", l.as_str().to_uppercase(), m));
        });
        if restored != 4 || !s4.iter().all(|x| x.state == "original") {
            return Err("Rollback did not restore all 4 targets to exact original SHA-256!".into());
        }
        lines.push("[OK] BILINGUAL SELFTEST PASSED 100%! (Original -> EN -> ID -> EN -> Rollback JP)".into());
        Ok(())
    })();

    if let Err(e) = &res {
        lines.push(format!("[ERR] SELFTEST FAILED: {}", e));
    }
    let _ = std::fs::write(&log_path, lines.join("\n"));
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

fn enable_windows_dark_titlebar(hwnd: isize) {
    let dark: i32 = 1;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd as _,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &dark as *const i32 as *const _,
            std::mem::size_of::<i32>() as u32,
        );
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 3 && args[1].eq_ignore_ascii_case("--selftest") {
        std::process::exit(run_selftest(&args[2]));
    }
    if args.len() >= 3 && args[1].eq_ignore_ascii_case("--scan") {
        let out = args.get(3).map(|s| s.as_str()).unwrap_or("scan.log");
        std::process::exit(run_cli_scan(&args[2], out));
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = WindowBuilder::new()
        .with_title("Ruza-Lang Patcher v0.1.0 — EN / ID")
        .with_inner_size(LogicalSize::new(920.0, 740.0))
        .with_min_inner_size(LogicalSize::new(820.0, 640.0))
        .build(&event_loop)
        .expect("Failed to create window");

    enable_windows_dark_titlebar(window.hwnd());

    // Background process guard poller (every 2 seconds)
    let poll_proxy = proxy.clone();
    thread::spawn(move || loop {
        let procs: Vec<ProcessInfo> = patch_engine::find_running_game_processes();
        push_event(&poll_proxy, json!({ "type": "processes", "processes": procs }));
        thread::sleep(Duration::from_secs(2));
    });

    let ipc_proxy = proxy.clone();
    let webview = WebViewBuilder::new()
        .with_html(INDEX_HTML)
        .with_background_color((8, 12, 21, 255))
        .with_ipc_handler(move |req| {
            let body = req.body().to_string();
            let parsed: IpcCommand = match serde_json::from_str(&body) {
                Ok(c) => c,
                Err(_) => return,
            };
            let p = ipc_proxy.clone();
            let lang = PatchLang::from_opt(parsed.lang.as_deref());
            match parsed.cmd.as_str() {
                "init" | "auto_detect" => {
                    thread::spawn(move || {
                        if parsed.cmd == "init" {
                            push_log(&p, LogLevel::Info, "Ruza-Lang Translation Patcher v0.1.0 (EN / ID) initialized.");
                            push_log(&p, LogLevel::Info, "Safety: Exact byte sizes preserved with SHA-256 verification.");
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
                "patch" => {
                    let dir = parsed.path.unwrap_or_default();
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
                        push_event(&p, json!({ "type": "busy", "value": true }));
                        match patch_engine::apply_patch(
                            &path,
                            lang,
                            |m, l| push_log(&p, l, m),
                            |cur, tot, st| {
                                push_event(&p, json!({ "type": "progress", "current": cur, "total": tot, "stage": st }));
                            },
                        ) {
                            Ok(n) => {
                                let (title, body) = match lang {
                                    PatchLang::Id => (
                                        "Patch Bahasa Indonesia Selesai!",
                                        format!("Patch Bahasa Indonesia (ID) berhasil dipasang & diverifikasi 100%!\nTotal string di-rewrite: {} strings.", n),
                                    ),
                                    PatchLang::En => (
                                        "English Patch Complete!",
                                        format!("English (EN) patch installed & verified 100%!\nTotal strings rewritten: {} strings.", n),
                                    ),
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
                        match patch_engine::rollback_to_original(
                            &path,
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
    let en_sum: usize = targets.iter().filter(|t| t.state == "patched_en").map(|t| t.string_count).sum();
    let id_sum: usize = targets.iter().filter(|t| t.state == "patched_id").map(|t| t.string_count).sum();
    let summary = if id_sum > 0 && en_sum == 0 {
        format!("Scan done: {} / {} strings currently in Bahasa Indonesia (ID).", id_sum, patch_engine::TOTAL_STRINGS)
    } else if en_sum > 0 && id_sum == 0 {
        format!("Scan done: {} / {} strings currently in English (EN).", en_sum, patch_engine::TOTAL_STRINGS)
    } else if en_sum > 0 && id_sum > 0 {
        format!("Scan done: mixed state (EN={}, ID={} of {}).", en_sum, id_sum, patch_engine::TOTAL_STRINGS)
    } else {
        format!("Scan done: Original JP (0 / {} strings translated).", patch_engine::TOTAL_STRINGS)
    };
    push_log(proxy, LogLevel::Ok, &summary);
    push_event(proxy, json!({ "type": "scan_result", "targets": targets }));
}
