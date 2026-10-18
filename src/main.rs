#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

//! Transcribe Studio — a desktop front end for faster-whisper and markitdown.
//!
//! Rust owns the window, the settings, the model library and every child
//! process; Python is reduced to two short-lived workers that speak JSON.

mod i18n;
mod jobs;
mod models;
mod paths;
mod power;
mod pyenv;
mod settings;

use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use jobs::{JobSummary, Jobs};
use models::ModelState;
use pyenv::EnvStatus;
use settings::{Settings, SettingsStore};

/// Containers the transcription tab accepts. faster-whisper decodes through
/// PyAV, so anything FFmpeg understands works — these are just the ones worth
/// offering in the file dialog.
const MEDIA_EXTENSIONS: &[&str] = &[
    "m4a", "mp3", "wav", "flac", "ogg", "opus", "aac", "wma", "aiff", "amr", "mp4", "mkv", "mov",
    "avi", "webm", "m4v", "mpeg", "mpg", "3gp", "ts",
];

/// What markitdown can turn into Markdown.
const DOCUMENT_EXTENSIONS: &[&str] = &[
    "pdf", "docx", "xlsx", "xls", "pptx", "html", "htm", "csv", "json", "xml", "txt", "md", "epub",
    "msg", "zip", "rss", "atom", "ipynb", "jpg", "jpeg", "png", "gif", "webp", "tiff", "bmp",
    "wav", "mp3", "m4a",
];

struct AppState {
    settings: SettingsStore,
    jobs: Arc<Jobs>,
}

/// Anything the window needs on first paint, in one round trip.
#[derive(Serialize)]
struct Bootstrap {
    settings: Settings,
    models: Vec<ModelState>,
    env: EnvStatus,
    machine: Machine,
    data_dir: String,
    media_extensions: Vec<String>,
    document_extensions: Vec<String>,
}

#[derive(Serialize)]
struct Machine {
    cpu_threads: usize,
    /// Suggested thread count: leaving a couple of cores free keeps the
    /// machine usable during a multi-hour run.
    suggested_threads: usize,
    /// Installed memory, rounded to one decimal.
    ram_gb: f64,
    /// Model id that suits this hardware — see `models::recommend`.
    recommended_model: &'static str,
}

#[derive(Serialize)]
struct FileInfo {
    path: String,
    name: String,
    ext: String,
    size: u64,
    /// `media`, `document` or `other` — drives the icon and which tab accepts it.
    kind: String,
}

#[cfg(windows)]
fn total_ram_gb() -> f64 {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
    status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
    if unsafe { GlobalMemoryStatusEx(&mut status) } == 0 {
        // Unknown memory should not push the user to a weaker model.
        return 16.0;
    }
    (status.ullTotalPhys as f64 / (1u64 << 30) as f64 * 10.0).round() / 10.0
}

#[cfg(not(windows))]
fn total_ram_gb() -> f64 {
    16.0
}

fn machine() -> Machine {
    let cpu_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let ram_gb = total_ram_gb();
    Machine {
        suggested_threads: cpu_threads.saturating_sub(2).max(2),
        recommended_model: models::recommend(cpu_threads, ram_gb),
        cpu_threads,
        ram_gb,
    }
}

fn classify(ext: &str) -> &'static str {
    let lower = ext.to_ascii_lowercase();
    if MEDIA_EXTENSIONS.contains(&lower.as_str()) {
        "media"
    } else if DOCUMENT_EXTENSIONS.contains(&lower.as_str()) {
        "document"
    } else {
        "other"
    }
}

fn describe(path: &Path) -> FileInfo {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    FileInfo {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        kind: classify(&ext).to_string(),
        ext,
        size: path.metadata().map(|m| m.len()).unwrap_or(0),
    }
}

#[tauri::command]
fn bootstrap(state: State<'_, AppState>) -> Bootstrap {
    Bootstrap {
        settings: state.settings.get(),
        models: models::states(),
        env: pyenv::resolve(),
        machine: machine(),
        data_dir: paths::data_root().to_string_lossy().into_owned(),
        media_extensions: MEDIA_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
        document_extensions: DOCUMENT_EXTENSIONS.iter().map(|s| s.to_string()).collect(),
    }
}

#[tauri::command]
fn save_settings(state: State<'_, AppState>, next: Settings) -> Result<(), String> {
    state.settings.set(next).map_err(|e| e.to_string())
}

#[tauri::command]
fn inspect_files(files: Vec<String>) -> Vec<FileInfo> {
    files
        .into_iter()
        .map(|f| describe(Path::new(&f)))
        .collect()
}

/// Opens a native file picker on a blocking thread — the dialog runs its own
/// modal loop and must not sit on the async runtime.
async fn pick(label: &'static str, extensions: &'static [&'static str]) -> Vec<String> {
    tokio::task::spawn_blocking(move || {
        rfd::FileDialog::new()
            .add_filter(label, extensions)
            .add_filter(i18n::t("filter.all"), &["*"])
            .pick_files()
            .map(|paths| {
                paths
                    .into_iter()
                    .map(|p| p.to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
async fn pick_media_files() -> Vec<String> {
    pick(i18n::t("filter.media"), MEDIA_EXTENSIONS).await
}

#[tauri::command]
async fn pick_document_files() -> Vec<String> {
    pick(i18n::t("filter.documents"), DOCUMENT_EXTENSIONS).await
}

#[tauri::command]
async fn pick_folder() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        rfd::FileDialog::new()
            .pick_folder()
            .map(|p| p.to_string_lossy().into_owned())
    })
    .await
    .ok()
    .flatten()
}

#[tauri::command]
fn list_models() -> Vec<ModelState> {
    models::states()
}

#[tauri::command]
async fn download_model(
    app: AppHandle,
    state: State<'_, AppState>,
    id: String,
) -> Result<(), String> {
    let jobs = state.jobs.clone();
    let flag = jobs.download_flag(&id);
    // A download can outlive several model switches, so the whole run is
    // driven here rather than detached — the UI awaits the result.
    let result = models::download(app, id.clone(), flag.clone()).await;
    jobs.clear_download(&id);
    result.map_err(|e| {
        if flag.load(Ordering::SeqCst) {
            i18n::t("err.download_cancelled").to_string()
        } else {
            e.to_string()
        }
    })
}

#[tauri::command]
fn cancel_model_download(state: State<'_, AppState>, id: String) {
    state.jobs.cancel_download(&id);
}

#[tauri::command]
fn delete_model(id: String) -> Result<(), String> {
    models::delete(&id).map_err(|e| e.to_string())
}

#[tauri::command]
fn env_status() -> EnvStatus {
    pyenv::resolve()
}

#[tauri::command]
async fn setup_env(app: AppHandle, state: State<'_, AppState>) -> Result<EnvStatus, String> {
    let source = state.settings.get().markitdown_source;
    pyenv::provision(app, source)
        .await
        .map_err(|e| e.to_string())?;
    Ok(pyenv::resolve())
}

#[tauri::command]
async fn start_transcribe(
    app: AppHandle,
    state: State<'_, AppState>,
    files: Vec<String>,
    settings: Settings,
) -> Result<JobSummary, String> {
    state.settings.set(settings.clone()).map_err(|e| e.to_string())?;

    if !models::is_installed(&settings.model) {
        let name = models::spec(&settings.model)
            .map(|s| s.name.to_string())
            .unwrap_or_else(|| settings.model.clone());
        return Err(i18n::tf("err.model_missing", &[("name", &name)]));
    }

    let model_path = models::model_path(&settings.model);
    let jobs = state.jobs.clone();
    jobs::transcribe(app, jobs, files, settings, model_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cancel_transcribe(state: State<'_, AppState>) -> Result<(), String> {
    state.jobs.cancel_transcribe().await;
    Ok(())
}

#[tauri::command]
async fn start_markdown(
    app: AppHandle,
    state: State<'_, AppState>,
    files: Vec<String>,
    output_dir: String,
) -> Result<Vec<String>, String> {
    let mut current = state.settings.get();
    current.md_output_dir = output_dir.clone();
    let _ = state.settings.set(current.clone());

    let env = pyenv::resolve();
    let ocr_langs = env.ready_ocr.then(|| {
        jobs::ocr_languages(&[&current.ui_language, &current.language], &env.ocr_languages)
    });

    let jobs = state.jobs.clone();
    jobs::markdown(app, jobs, files, output_dir, ocr_langs)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn cancel_markdown(state: State<'_, AppState>) -> Result<(), String> {
    state.jobs.cancel_markdown().await;
    Ok(())
}

#[tauri::command]
fn reveal_path(path: String) -> Result<(), String> {
    jobs::reveal(Path::new(&path)).map_err(|e| e.to_string())
}

/// Reads a produced Markdown file back for the preview pane.
#[tauri::command]
fn read_text(path: String, limit: usize) -> Result<String, String> {
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.chars().take(limit.max(1_000)).collect())
}

/// Appends a line to the client log. The window reports its own unhandled
/// errors here, so a problem that only shows up on a user's machine leaves a
/// trace instead of vanishing with the webview.
#[tauri::command]
fn log_client(level: String, message: String) {
    use std::io::Write;
    let _ = std::fs::create_dir_all(paths::log_dir());
    let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(paths::log_dir().join("client.log"))
    {
        let _ = writeln!(file, "[{stamp}] {level}: {message}");
    }
}

/// Called once the first paint is done. The window starts hidden so the user
/// never sees an unstyled white flash while the webview boots.
#[tauri::command]
fn ui_ready(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Settings for this launch: resolves the interface language and, on the very
/// first launch, seeds defaults that depend on the machine and the language.
fn initial_settings() -> Settings {
    let (mut settings, fresh) = Settings::load();

    // The installer's language applies when nothing was chosen yet, and again
    // after a reinstall in a different language — but never overrides a
    // choice made in the app since the last install.
    let installer = i18n::installer_langid();
    let reinstalled = installer.is_some() && installer != settings.installer_langid;
    if i18n::normalize(&settings.ui_language).is_none() || reinstalled {
        settings.ui_language = i18n::detect().to_string();
    }
    settings.installer_langid = installer;

    if fresh {
        // Most people record in the language they use the computer in.
        settings.language = settings.ui_language.clone();
        settings.model = machine().recommended_model.to_string();
    }

    i18n::set(&settings.ui_language);
    let _ = settings.save();
    settings
}

fn main() {
    paths::ensure_dirs();
    models::sweep_partials();
    let settings = initial_settings();

    tauri::Builder::default()
        // Must be registered first so a second launch is intercepted before
        // any other setup runs.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .manage(AppState {
            settings: SettingsStore::new(settings),
            jobs: Arc::new(Jobs::default()),
        })
        .setup(|app| {
            if let Ok(dir) = app.path().resource_dir() {
                paths::set_resource_dir(dir);
            }
            if let Err(err) = jobs::install_workers() {
                // Not fatal at startup: the UI surfaces it when a job is run.
                eprintln!("could not unpack the workers: {err}");
            }

            #[cfg(debug_assertions)]
            if std::env::var_os("TS_DEVTOOLS").is_some() {
                if let Some(window) = app.get_webview_window("main") {
                    window.open_devtools();
                }
            }

            let handle = app.handle().clone();
            let _ = handle.emit("app:ready", ());
            Ok(())
        })
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                let jobs = window.state::<AppState>().jobs.clone();
                tauri::async_runtime::block_on(async move {
                    jobs.cancel_transcribe().await;
                    jobs.cancel_markdown().await;
                });
            }
        })
        .invoke_handler(tauri::generate_handler![
            bootstrap,
            save_settings,
            inspect_files,
            pick_media_files,
            pick_document_files,
            pick_folder,
            list_models,
            download_model,
            cancel_model_download,
            delete_model,
            env_status,
            setup_env,
            start_transcribe,
            cancel_transcribe,
            start_markdown,
            cancel_markdown,
            reveal_path,
            read_text,
            log_client,
            ui_ready,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Transcribe Studio");
}
