// plainfold desktop shell. Reads a project with the core and hands the story model to the view (`app/src`).
// It also remembers recent projects and opens files in the user's editor. It never writes to a project.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use plainfold_core::changes::{Change, Watch};
use plainfold_core::diff::StoryDiff;
use plainfold_core::english::{self, Batch, Provider, Summary};
use plainfold_core::keep::{Held, Kept};
use plainfold_core::model::ProjectModel;
use plainfold_core::servers::Setup;
use plainfold_core::{Need, NoServer, Progress, ServerState};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

#[derive(Serialize)]
struct Opened {
    path: String,
    model: ProjectModel,
    adapters: Vec<String>,
    servers: Vec<ServerState>,
    failed: Vec<NoServer>,
    missing: Vec<String>,
    /// what changed since the user last marked the changes as seen (empty the first time a project is read)
    diff: Vec<StoryDiff>,
}

/// One progress step of one `open_project` call; `id` lets the view ignore a read it no longer waits for.
#[derive(Serialize, Clone)]
struct ProgressEvent {
    id: u32,
    #[serde(flatten)]
    step: Progress,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Recent {
    path: String,
    name: String,
    stories: usize,
    adapters: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
struct State {
    recent: Vec<Recent>,
    show_welcome: bool,
    /// download the language servers a project needs without asking
    auto_download: bool,
    /// keep a project's language servers running between reads (`Kept`)
    keep_servers: bool,
    /// the steps in plain English, written by a model, where there is a key for one
    english: bool,
}

impl Default for State {
    fn default() -> Self {
        State { recent: vec![], show_welcome: true, auto_download: false, keep_servers: true, english: true }
    }
}

/// How long a window's language servers are kept while it is not in front and nothing has read with them.
/// `PLAINFOLD_KEEP_IDLE_SECS` shortens it, for development.
fn idle() -> Duration {
    std::env::var("PLAINFOLD_KEEP_IDLE_SECS").ok().and_then(|s| s.parse().ok()).map(Duration::from_secs).unwrap_or(Duration::from_secs(20 * 60))
}

/// What a window keeps running for its open project: only that project's servers, never another window's.
struct Window {
    kept: Option<Kept>,
    focused: bool,
    /// the last read, or the last time the window came to the front or went behind
    active: Instant,
    /// why its servers were last closed, and when (seconds since 1970)
    closed: Option<(String, u64)>,
}

static WINDOWS: Mutex<BTreeMap<String, Window>> = Mutex::new(BTreeMap::new());

fn now_secs() -> u64 {
    SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Runs `f` on a window's entry, made when first needed.
fn with_window<T>(label: &str, f: impl FnOnce(&mut Window) -> T) -> T {
    let mut all = WINDOWS.lock().unwrap();
    let w = all.entry(label.to_string()).or_insert_with(|| Window { kept: None, focused: true, active: Instant::now(), closed: None });
    f(w)
}

/// Stops servers away from the caller: a server asked to shut down may take a moment to answer.
fn stop_later(kept: Option<Kept>) {
    if let Some(k) = kept.filter(|k| !k.is_empty()) {
        std::thread::spawn(move || drop(k));
    }
}

/// Closes the servers one window keeps, saying why (nothing is said when there were none).
fn close_kept(label: &str, why: &str) {
    let kept = with_window(label, |w| {
        let k = w.kept.take();
        if k.as_ref().is_some_and(|k| !k.is_empty()) {
            w.closed = Some((why.to_string(), now_secs()));
        }
        k
    });
    stop_later(kept);
}

fn close_all_kept(why: &str) {
    let labels: Vec<String> = WINDOWS.lock().unwrap().keys().cloned().collect();
    for l in labels {
        close_kept(&l, why);
    }
}

/// Whether the system says memory is running low (macOS: the memory pressure level is "warn" or "critical").
fn memory_low() -> bool {
    if !cfg!(target_os = "macos") {
        return false;
    }
    let out = Command::new("sysctl").args(["-n", "kern.memorystatus_vm_pressure_level"]).output();
    out.ok().and_then(|o| String::from_utf8_lossy(&o.stdout).trim().parse::<u32>().ok()).is_some_and(|level| level >= 2)
}

/// Every half minute: closes the servers of windows left alone for `IDLE`, and every window's when memory runs low.
fn watch_kept() {
    loop {
        std::thread::sleep(Duration::from_secs(30));
        if memory_low() {
            close_all_kept("the computer was low on memory");
            continue;
        }
        let limit = idle();
        let away: Vec<String> = WINDOWS.lock().unwrap().iter().filter(|(_, w)| w.kept.is_some() && !w.focused && w.active.elapsed() >= limit).map(|(l, _)| l.clone()).collect();
        let minutes = limit.as_secs().div_ceil(60);
        for l in away {
            close_kept(&l, &format!("not used for {minutes} minute{}", if minutes == 1 { "" } else { "s" }));
        }
    }
}

/// What the settings show about one window's language servers.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServerSettings {
    keep: bool,
    /// the project they run for
    project: Option<String>,
    running: Vec<Held>,
    /// why they were last closed, and when (seconds since 1970)
    closed: Option<(String, u64)>,
}

#[tauri::command]
fn server_settings(app: AppHandle, window: WebviewWindow) -> ServerSettings {
    let keep = load_state(&app).keep_servers;
    with_window(window.label(), |w| ServerSettings {
        keep,
        project: w.kept.as_ref().filter(|k| !k.is_empty()).map(|k| k.root.to_string_lossy().to_string()),
        running: w.kept.as_ref().map(|k| k.held()).unwrap_or_default(),
        closed: w.closed.clone(),
    })
}

/// Off: every window's servers stop now, and each read starts its own.
#[tauri::command]
fn set_keep_servers(app: AppHandle, on: bool) -> Result<State, String> {
    if !on {
        close_all_kept("turned off in Settings");
    }
    update_state(&app, |s| s.keep_servers = on)
}

/// The window went back to the welcome screen: its project is closed, and so are its servers.
#[tauri::command]
fn close_project(window: WebviewWindow) {
    close_kept(window.label(), "the project was closed");
    stop_english_run();
}

static STATE: Mutex<()> = Mutex::new(());

/// The open project's watch. Replaced when a project is read; one project is open at a time.
static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

/// The open project's model as the user last saw it (marked as seen, or when it was first read), and as it is now.
/// Changes are what differs between the two, so those of several Reloads add up until they are marked as seen.
struct Seen {
    path: String,
    seen: ProjectModel,
    now: ProjectModel,
}
static SEEN: Mutex<Option<Seen>> = Mutex::new(None);

/// The user has seen the changes: what they compare with becomes the model as it is now.
#[tauri::command]
fn mark_seen(path: String) {
    if let Some(s) = SEEN.lock().unwrap().as_mut().filter(|s| s.path == path) {
        s.seen = s.now.clone();
    }
}

/// Which of the files read `id` built its stories from differ now (empty: none, again).
#[derive(Serialize, Clone)]
struct ChangesEvent {
    id: u32,
    changes: Vec<Change>,
}

/// Languages the user stopped waiting for, by read (`id`); "*" stops the whole read.
static STOPPED: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

/// Stops waiting for a language server in one read (`kind` "*": the whole read). That language then shows nothing.
#[tauri::command]
fn stop_waiting(id: u32, kind: String) {
    STOPPED.lock().unwrap().push((id, kind));
}

fn state_file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_data_dir().ok().map(|d| d.join("state.json"))
}

fn load_state(app: &AppHandle) -> State {
    state_file(app).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

fn update_state(app: &AppHandle, f: impl FnOnce(&mut State)) -> Result<State, String> {
    let _guard = STATE.lock().map_err(|e| e.to_string())?;
    let mut s = load_state(app);
    f(&mut s);
    let path = state_file(app).ok_or("no app data folder")?;
    std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&path, serde_json::to_string_pretty(&s).unwrap()).map_err(|e| e.to_string())?;
    Ok(s)
}

/// Where the language servers come from: on PATH, or downloaded into the app data folder when a project needs them.
/// `PLAINFOLD_SERVERS` / `PLAINFOLD_TSSERVER` point at other ones, for development.
fn setup(app: &AppHandle) -> Setup {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from).and_then(|p| std::path::absolute(&p).ok());
    Setup {
        servers: env("PLAINFOLD_SERVERS"),
        tsserver: env("PLAINFOLD_TSSERVER"),
        python: None,
        store: app.path().app_data_dir().ok().map(|d| d.join("servers")),
    }
}

/// The languages a project uses and whether each one's server is here, can be downloaded, or cannot run.
#[tauri::command]
async fn check_servers(app: AppHandle, path: String) -> Result<Vec<Need>, String> {
    let root = PathBuf::from(&path).canonicalize().map_err(|e| format!("{path}: {e}"))?;
    let setup = setup(&app);
    tauri::async_runtime::spawn_blocking(move || plainfold_core::needs(&root, &setup)).await.map_err(|e| e.to_string())
}

/// Downloads the servers for `kinds`, reporting progress as `progress` events with this `id`.
#[tauri::command]
async fn install_servers(app: AppHandle, id: u32, kinds: Vec<String>) -> Result<(), String> {
    let setup = setup(&app);
    let emitter = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        for kind in kinds {
            plainfold_core::install(&kind, &setup, &|step| {
                let _ = emitter.emit("progress", ProgressEvent { id, step });
            })?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn set_auto_download(app: AppHandle, on: bool) -> Result<State, String> {
    update_state(&app, |s| s.auto_download = on)
}

#[derive(Serialize)]
struct Startup {
    #[serde(flatten)]
    state: State,
    /// a folder given on the command line (`plainfold <folder>`), opened instead of the welcome window
    open: Option<String>,
}

#[tauri::command]
fn startup(app: AppHandle) -> Startup {
    let mut state = load_state(&app);
    state.recent.retain(|r| Path::new(&r.path).is_dir());
    let open = std::env::args().skip(1).find(|a| Path::new(a).is_dir());
    Startup { state, open }
}

#[tauri::command]
async fn open_project(app: AppHandle, window: WebviewWindow, id: u32, path: String) -> Result<Opened, String> {
    let root = PathBuf::from(&path).canonicalize().map_err(|e| format!("{path}: {e}"))?;
    if !root.is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    let emitter = app.clone();
    let dir = root.clone();
    let setup = setup(&app);
    let label = window.label().to_string();
    // the servers this window keeps: used again for the same project, stopped by the read for another one
    let kept = if load_state(&app).keep_servers { with_window(&label, |w| w.kept.take()) } else { None };
    let x = tauri::async_runtime::spawn_blocking(move || {
        let stopped = |kind: &str| STOPPED.lock().unwrap().iter().any(|(i, k)| *i == id && (k == kind || k == "*"));
        plainfold_core::read(
            &dir,
            &setup,
            &|step| {
                let _ = emitter.emit("progress", ProgressEvent { id, step });
            },
            &stopped,
            kept,
        )
    })
    .await;
    // the view ends a read it no longer waits for (another project, or the same one again)
    let superseded = STOPPED.lock().unwrap().iter().any(|(i, k)| *i == id && k == "*");
    STOPPED.lock().unwrap().retain(|(i, _)| *i != id);
    let (mut x, kept) = x.map_err(|e| e.to_string())?.map_err(|e| format!("{}: {e}", root.display()))?;
    if superseded || !load_state(&app).keep_servers {
        stop_later(Some(kept));
    } else {
        let old = with_window(&label, |w| {
            w.active = Instant::now();
            w.closed = None;
            w.kept.replace(kept)
        });
        stop_later(old);
    }
    if !superseded {
        let emitter = app.clone();
        let watch = plainfold_core::changes::watch(&root, std::mem::take(&mut x.hashes), move |changes| {
            let _ = emitter.emit("changes", ChangesEvent { id, changes });
        });
        // a project that cannot be watched still opens; Reload still reads it again
        *WATCH.lock().unwrap() = watch.inspect_err(|e| eprintln!("plainfold: not watching {}: {e}", root.display())).ok();
    }
    let path = root.to_string_lossy().to_string();
    let diff = {
        let mut seen = SEEN.lock().unwrap();
        match seen.as_mut().filter(|s| s.path == path && !superseded) {
            Some(s) => {
                s.now = x.model.clone();
                plainfold_core::diff::diff(&s.seen, &s.now)
            }
            None if superseded => vec![],
            None => {
                *seen = Some(Seen { path: path.clone(), seen: x.model.clone(), now: x.model.clone() });
                vec![]
            }
        }
    };
    let recent = Recent { path: path.clone(), name: x.model.name.clone(), stories: x.model.stories.len(), adapters: x.adapters.clone() };
    update_state(&app, |s| {
        s.recent.retain(|r| r.path != recent.path);
        s.recent.insert(0, recent);
        s.recent.truncate(10);
    })?;
    Ok(Opened { path, model: x.model, adapters: x.adapters, servers: x.servers, failed: x.failed, missing: x.missing, diff })
}

// ---------- English ----------
// The steps in plain English (core `english`). The key lives in the system's keychain and is sent only to the model's
// endpoint. What a model wrote is kept in the app data folder by the hash of what was asked, so the same code is never
// asked about twice.

const KEY_SERVICE: &str = "plainfold";
const KEY_ACCOUNT: &str = "openai-api-key";

/// The run for the open project, stopped when another starts or English is turned off.
static ENGLISH: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

fn key_entry() -> Option<keyring::Entry> {
    keyring::Entry::new(KEY_SERVICE, KEY_ACCOUNT).ok()
}

/// The key once read: the keychain is asked once while the app runs (the system may ask the user to allow it).
static KEY: Mutex<Option<Option<(String, &'static str)>>> = Mutex::new(None);

/// The key: the one saved in the keychain, else `OPENAI_API_KEY` (an app started from a terminal, for development).
fn api_key() -> Option<(String, &'static str)> {
    let mut known = KEY.lock().unwrap();
    known
        .get_or_insert_with(|| {
            if let Some(k) = key_entry().and_then(|e| e.get_password().ok()).filter(|k| !k.trim().is_empty()) {
                return Some((k, "keychain"));
            }
            std::env::var("OPENAI_API_KEY").ok().filter(|k| !k.trim().is_empty()).map(|k| (k, "environment"))
        })
        .clone()
}

#[derive(Serialize)]
struct KeyState {
    /// where the key comes from: "keychain", "environment", or none
    from: Option<&'static str>,
    /// the model the key is used with
    model: String,
}

fn key_state() -> KeyState {
    KeyState { from: api_key().map(|(_, from)| from), model: english::openai::OpenAi::with_key("").model() }
}

#[tauri::command]
fn english_key() -> KeyState {
    key_state()
}

/// Saves the key in the keychain, or removes it (`None`).
#[tauri::command]
fn set_english_key(key: Option<String>) -> Result<KeyState, String> {
    let entry = key_entry().ok_or("the keychain cannot be used")?;
    match key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty()) {
        Some(k) if k.contains(char::is_whitespace) => return Err("a key has no spaces in it".into()),
        Some(k) => entry.set_password(&k).map_err(|e| e.to_string())?,
        None => match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(e) => return Err(e.to_string()),
        },
    }
    // read again on next use
    *KEY.lock().unwrap() = None;
    Ok(key_state())
}

fn stop_english_run() {
    if let Some(stop) = ENGLISH.lock().unwrap().take() {
        stop.store(true, Ordering::Relaxed);
    }
}

#[tauri::command]
fn set_english(app: AppHandle, on: bool) -> Result<State, String> {
    if !on {
        stop_english_run();
    }
    update_state(&app, |s| s.english = on)
}

#[tauri::command]
fn stop_english() {
    stop_english_run();
}

/// Texts that arrived for read `id`.
#[derive(Serialize, Clone)]
struct EnglishEvent {
    id: u32,
    #[serde(flatten)]
    batch: Batch,
}

/// The run for read `id` ended.
#[derive(Serialize, Clone)]
struct EnglishEnd {
    id: u32,
    #[serde(flatten)]
    summary: Summary,
    stopped: bool,
}

/// Starts writing the English of the open project's steps, as `english` events for read `id` and one `english-end`.
/// Returns the model that writes it.
#[tauri::command]
fn describe(app: AppHandle, id: u32, path: String) -> Result<String, String> {
    stop_english_run();
    let (key, _) = api_key().ok_or("no API key")?;
    let model = SEEN.lock().unwrap().as_ref().filter(|s| s.path == path).map(|s| s.now.clone()).ok_or("the project is not open")?;
    let stop = Arc::new(AtomicBool::new(false));
    *ENGLISH.lock().unwrap() = Some(stop.clone());
    let keep = app.path().app_data_dir().ok().map(|d| d.join("english"));
    let provider = english::openai::OpenAi::with_key(&key);
    let name = provider.model();
    std::thread::spawn(move || {
        let chunks = english::plan(&model, &provider.model());
        let summary = english::describe(&chunks, &provider, keep.as_deref(), &stop, &|b| {
            let _ = app.emit("english", EnglishEvent { id, batch: b.clone() });
        });
        let _ = app.emit("english-end", EnglishEnd { id, summary, stopped: stop.load(Ordering::Relaxed) });
    });
    Ok(name)
}

#[tauri::command]
fn set_show_welcome(app: AppHandle, show: bool) -> Result<State, String> {
    update_state(&app, |s| s.show_welcome = show)
}

/// Opens a project file at a line in the user's editor: the first of the editors people keep open next to plainfold
/// whose command line tool is installed, else the system's default app for the file (without the line).
#[tauri::command]
fn open_in_editor(root: String, file: String, line: usize) -> Result<String, String> {
    let root = PathBuf::from(&root).canonicalize().map_err(|e| e.to_string())?;
    let abs = root.join(&file).canonicalize().map_err(|e| format!("{file}: {e}"))?;
    if !abs.starts_with(&root) {
        return Err(format!("{file} is outside the project"));
    }
    let at = format!("{}:{}", abs.display(), line.max(1));
    let editors: [(&str, Vec<String>); 3] = [("zed", vec![at.clone()]), ("cursor", vec!["-g".into(), at.clone()]), ("code", vec!["-g".into(), at])];
    for (name, args) in editors {
        if let Some(exe) = find_exe(name) {
            Command::new(exe).args(args).stdin(Stdio::null()).spawn().map_err(|e| e.to_string())?;
            return Ok(name.to_string());
        }
    }
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(&abs).spawn().map_err(|e| e.to_string())?;
    Ok("the default app".into())
}

fn find_exe(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) { vec![format!("{name}.cmd"), format!("{name}.exe")] } else { vec![name.to_string()] };
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| p.is_file())
}

/// An app started from the Finder or the Dock does not get the PATH a terminal has, so the language servers and the
/// runtimes they need would not be found. Ask the user's login shell once, before any thread starts.
fn adopt_login_path() {
    if !cfg!(unix) || std::env::var_os("TERM").is_some() {
        return; // started from a terminal: its PATH is already the user's
    }
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let Ok(mut child) = Command::new(shell)
        .args(["-ilc", "printf '\\n__PLAINFOLD_PATH__%s\\n' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return;
    };
    let t0 = Instant::now();
    while child.try_wait().ok().flatten().is_none() {
        if t0.elapsed() > Duration::from_secs(3) {
            let _ = child.kill();
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let Ok(out) = child.wait_with_output() else { return };
    let text = String::from_utf8_lossy(&out.stdout);
    if let Some(path) = text.lines().find_map(|l| l.strip_prefix("__PLAINFOLD_PATH__")).filter(|p| !p.is_empty()) {
        // SAFETY: called first thing in main, before any other thread exists
        unsafe { std::env::set_var("PATH", path) };
    }
}

fn main() {
    adopt_login_path();
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            std::thread::spawn(watch_kept);
            // downloaded servers and Node.js versions this version no longer runs
            let setup = setup(app.handle());
            std::thread::spawn(move || plainfold_core::prune(&setup));
            Ok(())
        })
        .on_window_event(|window, event| match event {
            tauri::WindowEvent::Focused(f) => with_window(window.label(), |w| {
                w.focused = *f;
                w.active = Instant::now();
            }),
            tauri::WindowEvent::Destroyed => {
                let kept = WINDOWS.lock().unwrap().remove(window.label()).and_then(|w| w.kept);
                stop_later(kept);
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            startup,
            check_servers,
            install_servers,
            open_project,
            close_project,
            mark_seen,
            stop_waiting,
            set_show_welcome,
            set_auto_download,
            set_keep_servers,
            server_settings,
            open_in_editor,
            english_key,
            set_english_key,
            set_english,
            stop_english,
            describe
        ])
        .build(tauri::generate_context!())
        .expect("plainfold could not start")
        .run(|_, event| {
            // quitting: no server outlives the app (they are stopped here, not on another thread)
            if let tauri::RunEvent::Exit = event {
                let all: Vec<Kept> = WINDOWS.lock().unwrap().values_mut().filter_map(|w| w.kept.take()).collect();
                drop(all);
            }
        });
}
