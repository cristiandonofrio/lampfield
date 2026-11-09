#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Lampfield window. A background thread watches Claude's files, rescans on change
//! and sends a snapshot to the window. There's no server: the page is embedded in the
//! binary and talks to this process over Tauri's IPC. Nothing here uses the network.

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent};
use tauri_plugin_opener::OpenerExt;
use lampfield_core::{alert_to_dismiss, archive_link, attention, default_paths, prune_dismissed, session_link, Attention, Paths, Scanner, Snapshot};

/// Lets several quick writes share one rescan.
const SETTLE: Duration = Duration::from_millis(300);
/// States also change with no file event: a quiet tool, or a turn going stale.
const TICK: Duration = Duration::from_secs(10);

const TRAY_ID: &str = "lampfield";
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_ALERT_ICON: &[u8] = include_bytes!("../icons/tray-alert.png");
/// Menu item ids for sessions start with this, then the session id.
const OPEN_PREFIX: &str = "open:";

#[derive(Default)]
struct Latest(Arc<Mutex<Option<Snapshot>>>);

/// Alerts you've waved off: session id to the alert key at the time. Kept in this app's
/// own data folder, never in Claude's.
struct Dismissed {
    map: Arc<Mutex<HashMap<String, String>>>,
    file: PathBuf,
}

/// Asks the watch thread for a rescan now rather than at the next tick.
struct Rescan(Mutex<Sender<()>>);

fn load_dismissed(file: &Path) -> HashMap<String, String> {
    std::fs::read(file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_dismissed(file: &Path, map: &HashMap<String, String>) {
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(file, serde_json::to_vec(map).unwrap_or_default()) {
        eprintln!("dismiss: could not save {}: {e}", file.display());
    }
}

#[tauri::command]
fn get_snapshot(latest: State<Latest>) -> Option<Snapshot> {
    latest.0.lock().unwrap().clone()
}

fn open_in_claude(app: &AppHandle, id: &str) -> Result<(), String> {
    let link = session_link(id).ok_or("not a local session id")?;
    let res = app.opener().open_url(link, None::<&str>).map_err(|e| e.to_string());
    println!("open: {id} ok={}", res.is_ok());
    res
}

#[tauri::command]
fn open_session(id: String, app: AppHandle) -> Result<(), String> {
    open_in_claude(&app, &id)
}

/// Opens a new Claude Code session with a prompt to archive these sessions.
#[tauri::command]
fn archive_in_claude(ids: Vec<String>, latest: State<Latest>, app: AppHandle) -> Result<(), String> {
    let picked: Vec<(String, String)> = {
        let latest = latest.0.lock().unwrap();
        let sessions: Vec<_> = latest.iter().flat_map(|s| &s.groups).flat_map(|g| &g.sessions).collect();
        ids.iter().filter_map(|id| sessions.iter().find(|s| &s.id == id).map(|s| (s.id.clone(), s.title.clone()))).collect()
    };
    let link = archive_link(&picked).ok_or("no sessions to archive")?;
    let res = app.opener().open_url(link, None::<&str>).map_err(|e| e.to_string());
    println!("archive: asked Claude about {} sessions, ok={}", picked.len(), res.is_ok());
    res
}

/// Quietens a session's "needs you" or error until something new happens in it.
#[tauri::command]
fn dismiss(id: String, latest: State<Latest>, dismissed: State<Dismissed>, rescan: State<Rescan>) -> Result<(), String> {
    let key = latest.0.lock().unwrap().as_ref().and_then(|s| alert_to_dismiss(s, &id)).ok_or("nothing to dismiss")?;
    let mut map = dismissed.map.lock().unwrap();
    map.insert(id.clone(), key);
    save_dismissed(&dismissed.file, &map);
    println!("dismiss: {id}");
    let _ = rescan.0.lock().unwrap().send(());
    Ok(())
}

#[tauri::command]
fn set_on_top(on: bool, window: WebviewWindow) -> Result<(), String> {
    window.set_always_on_top(on).map_err(|e| e.to_string())
}

/// The window reports what it drew, so a run can be checked from the terminal.
#[tauri::command]
fn report_rendered(groups: usize, tiles: usize, states: String) {
    println!("window: drew {groups} groups, {tiles} tiles ({states})");
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn short(title: &str) -> String {
    const MAX: usize = 44;
    if title.chars().count() <= MAX {
        return title.to_string();
    }
    format!("{}...", title.chars().take(MAX - 3).collect::<String>())
}

fn heading(now: &Attention) -> String {
    match (now.urgent.len(), now.quiet) {
        (0, 0) => "Nothing needs you".to_string(),
        (0, q) => format!("{q} may need you"),
        (1, _) => "1 needs you".to_string(),
        (n, _) => format!("{n} need you"),
    }
}

fn tray_menu(app: &AppHandle, now: &Attention) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::new(app, heading(now), false, None::<&str>)?)?;
    for (id, title) in &now.urgent {
        menu.append(&MenuItem::with_id(app, format!("{OPEN_PREFIX}{id}"), short(title), true, None::<&str>)?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "show", "Show Lampfield", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "quit", "Quit Lampfield", true, None::<&str>)?)?;
    Ok(menu)
}

fn update_tray(app: &AppHandle, now: &Attention) -> tauri::Result<()> {
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return Ok(()) };
    let urgent = !now.urgent.is_empty();
    tray.set_icon(Some(Image::from_bytes(if urgent { TRAY_ALERT_ICON } else { TRAY_ICON })?))?;
    // Windows has no text next to tray icons, so the count goes in the hover text there.
    tray.set_tooltip(Some(format!("Lampfield: {}", heading(now).to_lowercase())))?;
    #[cfg(target_os = "macos")]
    {
        tray.set_icon_as_template(true)?;
        let title = match (now.urgent.len(), now.quiet) {
            (0, 0) => String::new(),
            (0, _) => "?".to_string(),
            (n, _) => n.to_string(),
        };
        tray.set_title(Some(title.as_str()))?;
    }
    tray.set_menu(Some(tray_menu(app, now)?))?;
    println!("tray: urgent={} quiet={}", now.urgent.len(), now.quiet);
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn log_snapshot(s: &Snapshot) {
    let live: usize = s.groups.iter().map(|g| g.sessions.len()).sum();
    println!(
        "scan: {} records, {} archived hidden, {live} active, groups from {}{}",
        s.total_records,
        s.archived_hidden,
        s.group_source,
        if s.group_note.is_empty() { String::new() } else { format!(" ({})", s.group_note) },
    );
}

fn watch(app: AppHandle, paths: Paths, latest: Arc<Mutex<Option<Snapshot>>>, dismissed: Arc<Mutex<HashMap<String, String>>>, dismissed_file: PathBuf, tx: Sender<()>, rx: Receiver<()>) {
    let config = paths.desktop_config.clone();
    let config_dir = config.parent().map(Path::to_path_buf);
    let handler = move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }
        // The config's folder also holds caches, so only that one file counts there.
        let relevant = event.paths.iter().any(|p| p == &config || p.parent() != config_dir.as_deref());
        if relevant {
            let _ = tx.send(());
        }
    };
    let mut watcher = match RecommendedWatcher::new(handler, notify::Config::default()) {
        Ok(w) => Some(w),
        Err(e) => {
            eprintln!("watch: unavailable, using the {}s tick only: {e}", TICK.as_secs());
            None
        }
    };
    if let Some(w) = watcher.as_mut() {
        let config_parent = paths.desktop_config.parent().map(Path::to_path_buf);
        let mut targets = vec![(paths.sessions_root.clone(), RecursiveMode::Recursive), (paths.transcripts_root.clone(), RecursiveMode::Recursive)];
        if let Some(p) = config_parent {
            targets.push((p, RecursiveMode::NonRecursive));
        }
        for (path, mode) in targets {
            match w.watch(&path, mode) {
                Ok(()) => println!("watch: {}", path.display()),
                Err(e) => eprintln!("watch: failed on {}: {e}", path.display()),
            }
        }
    }

    let mut scanner = Scanner::new(paths);
    let mut shown: Option<Attention> = None;
    loop {
        scanner.dismissed = dismissed.lock().unwrap().clone();
        let snap = scanner.snapshot(now_ms());
        {
            let mut map = dismissed.lock().unwrap();
            if prune_dismissed(&snap, &mut map) {
                save_dismissed(&dismissed_file, &map);
            }
        }
        let changed = latest.lock().unwrap().as_ref() != Some(&snap);
        if changed {
            log_snapshot(&snap);
            *latest.lock().unwrap() = Some(snap.clone());
            if let Err(e) = app.emit("snapshot", &snap) {
                eprintln!("emit failed: {e}");
            }
            let now = attention(&snap);
            if shown.as_ref() != Some(&now) {
                if let Err(e) = update_tray(&app, &now) {
                    eprintln!("tray update failed: {e}");
                }
                shown = Some(now);
            }
        }
        match rx.recv_timeout(TICK) {
            Ok(()) => {
                std::thread::sleep(SETTLE);
                while rx.try_recv().is_ok() {}
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => std::thread::sleep(TICK),
        }
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(Latest::default())
        .invoke_handler(tauri::generate_handler![get_snapshot, open_session, dismiss, archive_in_claude, set_on_top, report_rendered])
        .on_window_event(|window, event| {
            // Closing only hides the window, so the tray keeps counting.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .setup(|app| {
            let tray = TrayIconBuilder::with_id(TRAY_ID)
                .icon(Image::from_bytes(TRAY_ICON)?)
                .tooltip("Lampfield")
                .menu(&tray_menu(app.handle(), &Attention::default())?)
                // On Windows a left click should open the board, and a right click the menu.
                .show_menu_on_left_click(cfg!(target_os = "macos"))
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                        if !cfg!(target_os = "macos") {
                            show_window(tray.app_handle());
                        }
                    }
                })
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_window(app),
                    "quit" => app.exit(0),
                    id => {
                        if let Some(session) = id.strip_prefix(OPEN_PREFIX) {
                            let _ = open_in_claude(app, session);
                        }
                    }
                });
            #[cfg(target_os = "macos")]
            let tray = tray.icon_as_template(true);
            tray.build(app)?;

            let paths = default_paths().ok_or("could not find the Claude app's data folder")?;
            println!("paths: sessions={} transcripts={} config={}", paths.sessions_root.display(), paths.transcripts_root.display(), paths.desktop_config.display());
            let latest = app.state::<Latest>().0.clone();
            let data_dir = app.path().app_data_dir()?;
            let file = data_dir.join("dismissed.json");
            let map = Arc::new(Mutex::new(load_dismissed(&file)));
            let (tx, rx) = channel();
            app.manage(Dismissed { map: map.clone(), file: file.clone() });
            app.manage(Rescan(Mutex::new(tx.clone())));
            let handle = app.handle().clone();
            std::thread::spawn(move || watch(handle, paths, latest, map, file, tx, rx));
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Lampfield")
        .run(|app, event| {
            // Clicking the Dock icon brings a hidden window back on macOS.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_window(app);
            }
            let _ = (app, event);
        });
}
