//! Runs the Python workers and relays their event stream to the UI.
//!
//! One worker of each kind may run at a time. The worker is held in a slot so
//! that a cancel request can kill it without waiting for the read loop, and
//! every line it prints is forwarded verbatim to the window.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex as StdMutex};

use anyhow::{bail, Context, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Child;
use tokio::sync::Mutex;

use crate::i18n::{t, tf};
use crate::paths;
use crate::power::SleepBlocker;
use crate::pyenv;
use crate::settings::Settings;

const TRANSCRIBE_WORKER: &str = include_str!("../py/transcribe_worker.py");
const MARKDOWN_WORKER: &str = include_str!("../py/markdown_worker.py");

/// Channel the UI listens on for each worker kind.
pub const EVENT_TRANSCRIBE: &str = "job:transcribe";
pub const EVENT_MARKDOWN: &str = "job:markdown";
/// The markitdown pass chained after a transcription. It gets its own channel
/// because its file indices refer to transcripts, not to the Markdown tab's
/// own queue, and would otherwise re-label rows there.
pub const EVENT_TRANSCRIBE_MD: &str = "job:transcribe-md";

#[derive(Default)]
pub struct Jobs {
    transcribe: Mutex<Option<Child>>,
    markdown: Mutex<Option<Child>>,
    /// Cancellation flags for in-flight model downloads, keyed by model id.
    downloads: StdMutex<HashMap<String, Arc<AtomicBool>>>,
}

impl Jobs {
    pub fn download_flag(&self, id: &str) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        self.downloads
            .lock()
            .unwrap()
            .insert(id.to_string(), flag.clone());
        flag
    }

    pub fn cancel_download(&self, id: &str) {
        if let Some(flag) = self.downloads.lock().unwrap().get(id) {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    pub fn clear_download(&self, id: &str) {
        self.downloads.lock().unwrap().remove(id);
    }

    pub async fn cancel_transcribe(&self) {
        if let Some(child) = self.transcribe.lock().await.as_mut() {
            let _ = child.start_kill();
        }
    }

    pub async fn cancel_markdown(&self) {
        if let Some(child) = self.markdown.lock().await.as_mut() {
            let _ = child.start_kill();
        }
    }
}

/// Unpacks the worker scripts next to the app data. Done on every launch so a
/// new build never runs last version's worker.
pub fn install_workers() -> Result<()> {
    let dir = paths::runtime_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("transcribe_worker.py"), TRANSCRIBE_WORKER)?;
    std::fs::write(dir.join("markdown_worker.py"), MARKDOWN_WORKER)?;
    Ok(())
}

fn write_job_file(name: &str, job: &Value) -> Result<PathBuf> {
    let dir = paths::runtime_dir();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(job)?)?;
    Ok(path)
}

/// Outcome of one worker run.
struct WorkerRun {
    /// Every event object the worker emitted, so callers can chain on results.
    events: Vec<Value>,
    /// True when the worker was killed before reporting `all_done`.
    cancelled: bool,
}

/// Drives one worker process to completion, streaming its events to the UI as
/// they arrive.
async fn run_worker(
    app: &AppHandle,
    slot: &Mutex<Option<Child>>,
    channel: &str,
    script: PathBuf,
    job_file: PathBuf,
) -> Result<WorkerRun> {
    if slot.lock().await.is_some() {
        bail!(t("err.busy"));
    }

    let python = pyenv::require_python()?;
    let mut cmd = pyenv::python_command(&python);
    cmd.arg("-u")
        .arg(&script)
        .arg(&job_file)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .with_context(|| tf("err.spawn", &[("what", &script.display().to_string())]))?;

    let stdout = child.stdout.take().context("worker has no stdout")?;
    let stderr = child.stderr.take().context("worker has no stderr")?;
    *slot.lock().await = Some(child);

    // stderr is drained in parallel: a full pipe would otherwise block the
    // worker mid-transcription.
    let app_err = app.clone();
    let channel_err = channel.to_string();
    let stderr_task = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        let mut collected = Vec::new();
        while let Ok(Some(line)) = lines.next_line().await {
            if line.trim().is_empty() {
                continue;
            }
            collected.push(line.clone());
            let _ = app_err.emit(&channel_err, json!({ "t": "stderr", "msg": line }));
        }
        collected
    });

    let mut events = Vec::new();
    let mut lines = BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(trimmed) {
            Ok(value) => {
                let _ = app.emit(channel, value.clone());
                events.push(value);
            }
            // Anything a dependency prints to stdout behind our back.
            Err(_) => {
                let _ = app.emit(channel, json!({ "t": "log", "msg": trimmed }));
            }
        }
    }

    let stderr_lines = stderr_task.await.unwrap_or_default();
    let status = match slot.lock().await.take() {
        Some(mut child) => child.wait().await.ok(),
        None => None,
    };

    let code = status.and_then(|s| s.code());
    let completed = events.iter().any(|e| e["t"] == "all_done");

    if !completed {
        // The worker diagnoses its own failures; prefer that message over an
        // exit code the user cannot act on.
        let reported = events
            .iter()
            .rev()
            .find(|e| e["t"] == "fatal")
            .and_then(|e| e["msg"].as_str().map(String::from));

        // A killed worker is a user-initiated cancel, not a failure.
        if reported.is_none() && (code.is_none() || code == Some(1) && stderr_lines.is_empty()) {
            let _ = app.emit(channel, json!({ "t": "cancelled" }));
            return Ok(WorkerRun { events, cancelled: true });
        }

        let message = reported.unwrap_or_else(|| {
            let detail = stderr_lines
                .iter()
                .rev()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(" / ");
            if detail.is_empty() {
                tf("err.worker_exit", &[("code", &format!("{code:?}"))])
            } else {
                detail
            }
        });
        bail!(message);
    }

    Ok(WorkerRun { events, cancelled: false })
}

#[derive(Serialize, Clone)]
pub struct JobSummary {
    pub outputs: Vec<String>,
    pub failures: u64,
    pub markdown_outputs: Vec<String>,
    /// Set when the run was stopped by the user, so the window does not
    /// announce a partial result as a success.
    pub cancelled: bool,
}

/// Transcribes `files` with the model and options from `settings`.
pub async fn transcribe(
    app: AppHandle,
    jobs: Arc<Jobs>,
    files: Vec<String>,
    settings: Settings,
    model_path: PathBuf,
) -> Result<JobSummary> {
    if files.is_empty() {
        bail!(t("err.no_files"));
    }

    // Held for the whole run so the machine cannot suspend between segments.
    let _wake = SleepBlocker::acquire(settings.prevent_sleep);
    let _ = app.emit(
        EVENT_TRANSCRIBE,
        json!({ "t": "wake_lock", "active": SleepBlocker::is_active() }),
    );

    let output_dir = if settings.output_dir.trim().is_empty() {
        Value::Null
    } else {
        Value::String(settings.output_dir.clone())
    };

    let job = json!({
        "files": files,
        "model_path": model_path.to_string_lossy(),
        "model_id": settings.model,
        "device": "cpu",
        "compute_type": settings.compute_type,
        "language": settings.language,
        "task": settings.task,
        "beam_size": settings.beam_size,
        "vad": settings.vad,
        "cpu_threads": settings.cpu_threads,
        "initial_prompt": settings.initial_prompt,
        "output_dir": output_dir,
        "formats": settings.formats,
        "skip_existing": settings.skip_existing,
        "ui_language": crate::i18n::current(),
    });

    let job_file = write_job_file("transcribe_job.json", &job)?;
    let script = paths::runtime_dir().join("transcribe_worker.py");

    let run = run_worker(&app, &jobs.transcribe, EVENT_TRANSCRIBE, script, job_file).await?;
    let events = &run.events;

    let mut outputs = Vec::new();
    let mut failures = 0u64;
    for event in events {
        match event["t"].as_str() {
            Some("file_done") => {
                if let Some(list) = event["outputs"].as_array() {
                    outputs.extend(list.iter().filter_map(|v| v.as_str().map(String::from)));
                }
            }
            Some("file_error") => failures += 1,
            Some("all_done") => {
                failures = failures.max(event["failures"].as_u64().unwrap_or(0));
            }
            _ => {}
        }
    }

    // Optional second pass: push the plain transcript through markitdown, for
    // when the user wants that exact pipeline rather than the native writer.
    let mut markdown_outputs = Vec::new();
    if settings.run_markitdown && !run.cancelled {
        let transcripts: Vec<String> = outputs
            .iter()
            .filter(|p| p.to_ascii_lowercase().ends_with(".txt"))
            .cloned()
            .collect();
        if !transcripts.is_empty() {
            let _ = app.emit(
                EVENT_TRANSCRIBE,
                json!({ "t": "log", "msg": tf("log.markitdown_files", &[("n", &transcripts.len().to_string())]) }),
            );
            match markdown_on(
                app.clone(),
                jobs.clone(),
                transcripts,
                settings.output_dir.clone(),
                true,
                EVENT_TRANSCRIBE_MD,
                None,
            )
            .await
            {
                Ok(list) => markdown_outputs = list,
                Err(err) => {
                    let _ = app.emit(
                        EVENT_TRANSCRIBE,
                        json!({ "t": "log", "msg": tf("log.markitdown_failed", &[("err", &err.to_string())]) }),
                    );
                }
            }
        }
    }

    Ok(JobSummary {
        outputs,
        failures,
        markdown_outputs,
        cancelled: run.cancelled,
    })
}

/// Tesseract language codes for the given interface/recording languages,
/// most relevant first, limited to the data actually shipped. English is
/// always included: it turns up in almost every document.
pub fn ocr_languages(wanted: &[&str], available: &[String]) -> String {
    let mut out: Vec<&str> = Vec::new();
    for lang in wanted.iter().chain(std::iter::once(&"en")) {
        let code = match *lang {
            "en" => "eng",
            "ru" => "rus",
            "es" => "spa",
            "fr" => "fra",
            _ => continue,
        };
        if available.iter().any(|a| a == code) && !out.contains(&code) {
            out.push(code);
        }
    }
    out.join("+")
}

/// Converts arbitrary documents to Markdown through markitdown, reading
/// scans with the bundled Tesseract when `ocr_langs` is given.
pub async fn markdown(
    app: AppHandle,
    jobs: Arc<Jobs>,
    files: Vec<String>,
    output_dir: String,
    ocr_langs: Option<String>,
) -> Result<Vec<String>> {
    markdown_on(app, jobs, files, output_dir, false, EVENT_MARKDOWN, ocr_langs).await
}

/// Same, but reporting on a caller-chosen channel.
async fn markdown_on(
    app: AppHandle,
    jobs: Arc<Jobs>,
    files: Vec<String>,
    output_dir: String,
    overwrite: bool,
    channel: &str,
    ocr_langs: Option<String>,
) -> Result<Vec<String>> {
    if files.is_empty() {
        bail!(t("err.no_files"));
    }

    let ocr = match (paths::tesseract_dir(), ocr_langs) {
        (Some(dir), Some(langs)) if !langs.is_empty() => json!({
            "tesseract": dir.join("tesseract.exe").to_string_lossy(),
            "tessdata": dir.join("tessdata").to_string_lossy(),
            "langs": langs,
        }),
        _ => Value::Null,
    };

    let job = json!({
        "files": files,
        "output_dir": if output_dir.trim().is_empty() { Value::Null } else { Value::String(output_dir) },
        "overwrite": overwrite,
        "enable_plugins": false,
        "ocr": ocr,
        "ui_language": crate::i18n::current(),
    });

    let job_file = write_job_file("markdown_job.json", &job)?;
    let script = paths::runtime_dir().join("markdown_worker.py");

    let run = run_worker(&app, &jobs.markdown, channel, script, job_file).await?;

    Ok(run
        .events
        .iter()
        .filter(|e| e["t"] == "file_done")
        .filter_map(|e| e["output"].as_str().map(String::from))
        .collect())
}

/// Opens Explorer with the given file selected, or the folder itself.
pub fn reveal(path: &Path) -> Result<()> {
    let mut cmd = std::process::Command::new("explorer.exe");
    if path.is_dir() {
        cmd.arg(path);
    } else {
        cmd.arg(format!("/select,{}", path.display()));
    }
    // Explorer reports a non-zero exit code even on success, so the status is
    // deliberately ignored.
    let _ = cmd.spawn()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ocr_languages;

    #[test]
    fn ocr_languages_follow_preferences() {
        let all: Vec<String> = ["eng", "fra", "rus", "spa"].iter().map(|s| s.to_string()).collect();
        assert_eq!(ocr_languages(&["ru", "ru"], &all), "rus+eng");
        assert_eq!(ocr_languages(&["fr", "es"], &all), "fra+spa+eng");
        assert_eq!(ocr_languages(&["en", "auto"], &all), "eng");
        assert_eq!(ocr_languages(&["de"], &all), "eng");
        assert_eq!(ocr_languages(&["ru"], &["eng".to_string()]), "eng");
    }
}
