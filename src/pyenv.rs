//! Discovery and provisioning of the Python environment the workers run in.
//!
//! Transcription rides on faster-whisper and Markdown conversion on
//! markitdown, both of which are Python. The app keeps that dependency out of
//! sight: it finds a usable interpreter, reports exactly what is missing, and
//! can build a private virtual environment on request.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};

use crate::i18n::{t, tf};
use crate::paths;

/// Suppresses the console window that would otherwise flash up for every
/// child process started from a windowed binary.
#[cfg(windows)]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Extras installed alongside markitdown. Deliberately excludes the
/// `azure-*` cloud converters from `[all]`, which add hundreds of megabytes
/// for features that need API keys nobody has configured.
const MARKITDOWN_EXTRAS: &str = "pptx,docx,xlsx,xls,pdf,outlook,audio-transcription";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Probe {
    pub python: Option<String>,
    pub exe: Option<String>,
    pub faster_whisper: Option<String>,
    pub markitdown: Option<String>,
    pub ctranslate2: Option<String>,
    pub av: Option<String>,
    /// Renders PDF pages to images for OCR.
    pub pypdfium2: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EnvStatus {
    /// Interpreter the workers will actually use, if one was found.
    pub python: Option<String>,
    /// `managed` or `none` — whether the app's own environment exists.
    pub source: String,
    pub probe: Probe,
    pub ready_transcribe: bool,
    pub ready_markdown: bool,
    /// Bundled Tesseract folder, absent in a portable copy.
    pub tesseract: Option<String>,
    /// Language data shipped with that Tesseract, e.g. `["eng", "rus"]`.
    pub ocr_languages: Vec<String>,
    /// Scans can be read: Tesseract is bundled and pypdfium2 is installed.
    pub ready_ocr: bool,
    pub managed_venv: String,
}

const PROBE_SRC: &str = r#"
import json, sys
try:
    import importlib.metadata as md
except Exception:
    md = None
def v(name):
    if md is None:
        return None
    try:
        return md.version(name)
    except Exception:
        return None
print(json.dumps({
    "python": sys.version.split()[0],
    "exe": sys.executable,
    "faster_whisper": v("faster-whisper"),
    "markitdown": v("markitdown"),
    "ctranslate2": v("ctranslate2"),
    "av": v("av"),
    "pypdfium2": v("pypdfium2"),
}))
"#;

/// Interpreters worth probing: only the environment the app owns, so every
/// dependency lives in the app's data folder.
fn candidates() -> Vec<(PathBuf, &'static str)> {
    vec![(paths::venv_python(&paths::venv_dir()), "managed")]
}

fn probe(python: &Path) -> Option<Probe> {
    if !python.is_file() {
        return None;
    }
    let mut cmd = std::process::Command::new(python);
    cmd.arg("-c")
        .arg(PROBE_SRC)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Finds the interpreter the workers should use. Prefers the app's own venv so
/// that provisioning is reproducible, then falls back to an environment the
/// user already set up by hand.
pub fn resolve() -> EnvStatus {
    let tesseract = paths::tesseract_dir();
    let ocr_languages = tesseract
        .as_ref()
        .and_then(|dir| std::fs::read_dir(dir.join("tessdata")).ok())
        .map(|entries| {
            let mut langs: Vec<String> = entries
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .strip_suffix(".traineddata")
                        .map(String::from)
                })
                .collect();
            langs.sort();
            langs
        })
        .unwrap_or_default();
    let tesseract = tesseract.map(|dir| dir.to_string_lossy().into_owned());

    for (python, source) in candidates() {
        if let Some(probe) = probe(&python) {
            return EnvStatus {
                python: Some(python.to_string_lossy().into_owned()),
                source: source.to_string(),
                ready_transcribe: probe.faster_whisper.is_some(),
                ready_markdown: probe.markitdown.is_some(),
                ready_ocr: tesseract.is_some() && probe.pypdfium2.is_some(),
                tesseract,
                ocr_languages,
                probe,
                managed_venv: paths::venv_dir().to_string_lossy().into_owned(),
            };
        }
    }
    EnvStatus {
        python: None,
        source: "none".into(),
        probe: Probe::default(),
        ready_transcribe: false,
        ready_markdown: false,
        tesseract,
        ocr_languages,
        ready_ocr: false,
        managed_venv: paths::venv_dir().to_string_lossy().into_owned(),
    }
}

/// The interpreter to launch workers with, or an actionable error.
pub fn require_python() -> Result<PathBuf> {
    match resolve().python {
        Some(p) => Ok(PathBuf::from(p)),
        None => bail!(t("err.env_missing")),
    }
}

/// A `Command` prepared for running one of the worker scripts.
pub fn python_command(python: &Path) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(python);
    cmd.env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .env("PYTHONUNBUFFERED", "1")
        // Hugging Face telemetry would add a pointless startup delay for a
        // tool that already has every model on disk.
        .env("HF_HUB_DISABLE_TELEMETRY", "1");
    #[cfg(windows)]
    {
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn log(app: &AppHandle, line: impl Into<String>) {
    let _ = app.emit("env:log", line.into());
}

/// Runs a child process, forwarding both streams to the UI log.
async fn run_logged(app: &AppHandle, mut cmd: tokio::process::Command, label: &str) -> Result<()> {
    use tokio::io::{AsyncBufReadExt, BufReader};

    log(app, format!("$ {label}"));
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .with_context(|| tf("err.spawn", &[("what", label)]))?;

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let app_out = app.clone();
    let app_err = app.clone();

    let t_out = tokio::spawn(async move {
        if let Some(s) = stdout {
            let mut lines = BufReader::new(s).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log(&app_out, line);
            }
        }
    });
    let t_err = tokio::spawn(async move {
        if let Some(s) = stderr {
            let mut lines = BufReader::new(s).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                log(&app_err, line);
            }
        }
    });

    let status = child.wait().await?;
    let _ = tokio::join!(t_out, t_err);

    if !status.success() {
        bail!(tf("err.step_failed", &[("what", label)]));
    }
    Ok(())
}

/// Locates a base interpreter able to create a virtual environment.
fn base_python() -> Result<Vec<String>> {
    // `py` is the Windows launcher; asking it for a specific minor version is
    // the only reliable way to avoid a Python that a dependency rejects.
    for version in ["3.12", "3.13", "3.11", "3.14"] {
        let mut cmd = std::process::Command::new("py");
        cmd.args([&format!("-{version}"), "-c", "import sys"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        if matches!(cmd.status(), Ok(s) if s.success()) {
            return Ok(vec!["py".into(), format!("-{version}")]);
        }
    }

    let mut cmd = std::process::Command::new("python");
    cmd.args(["-c", "import sys"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    if matches!(cmd.status(), Ok(s) if s.success()) {
        return Ok(vec!["python".into()]);
    }

    bail!(t("err.no_python"))
}

/// Creates the managed virtual environment if needed and installs whatever is
/// missing. Safe to re-run — it is also the "repair" path.
pub async fn provision(app: AppHandle, markitdown_source: String) -> Result<()> {
    paths::ensure_dirs();

    let mut status = resolve();

    // Only build a new venv when there is nothing usable at all; an
    // environment the user already has keeps working.
    if status.python.is_none() {
        let base = base_python()?;
        log(&app, t("log.creating_venv"));
        let venv = paths::venv_dir();
        let mut cmd = tokio::process::Command::new(&base[0]);
        for a in &base[1..] {
            cmd.arg(a);
        }
        cmd.arg("-m").arg("venv").arg(&venv);
        #[cfg(windows)]
        {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        run_logged(&app, cmd, &format!("{} -m venv", base.join(" "))).await?;

        status = resolve();
        if status.python.is_none() {
            bail!(t("err.venv_dead"));
        }
    }

    let python = PathBuf::from(status.python.clone().unwrap());
    log(
        &app,
        tf(
            "log.env",
            &[
                ("path", &python.display().to_string()),
                ("version", &status.probe.python.clone().unwrap_or_else(|| "?".into())),
            ],
        ),
    );

    let mut pip = python_command(&python);
    pip.args(["-m", "pip", "install", "--upgrade", "pip"]);
    // A stale pip is a nuisance, not a blocker.
    let _ = run_logged(&app, pip, "pip install --upgrade pip").await;

    if status.probe.faster_whisper.is_none() {
        log(&app, tf("log.installing", &[("what", "faster-whisper")]));
        let mut cmd = python_command(&python);
        cmd.args(["-m", "pip", "install", "faster-whisper", "av"]);
        run_logged(&app, cmd, "pip install faster-whisper av").await?;
    } else {
        log(
            &app,
            tf(
                "log.already",
                &[("what", "faster-whisper"), ("version", &status.probe.faster_whisper.clone().unwrap())],
            ),
        );
    }

    if status.probe.markitdown.is_none() {
        // A local checkout is preferred when the user has one: it pins the
        // exact version they looked at instead of whatever PyPI serves today.
        let target = resolve_markitdown_target(&markitdown_source);
        log(&app, tf("log.installing_from", &[("source", &target)]));
        let mut cmd = python_command(&python);
        cmd.args(["-m", "pip", "install", &target]);
        run_logged(&app, cmd, "pip install markitdown").await?;
    } else {
        log(
            &app,
            tf(
                "log.already",
                &[("what", "markitdown"), ("version", &status.probe.markitdown.clone().unwrap())],
            ),
        );
    }

    if status.probe.pypdfium2.is_none() {
        log(&app, tf("log.installing", &[("what", "pypdfium2")]));
        let mut cmd = python_command(&python);
        cmd.args(["-m", "pip", "install", "pypdfium2", "pillow"]);
        // Only scan recognition needs it; everything else keeps working.
        if let Err(err) = run_logged(&app, cmd, "pip install pypdfium2 pillow").await {
            log(&app, err.to_string());
        }
    } else {
        log(
            &app,
            tf(
                "log.already",
                &[("what", "pypdfium2"), ("version", &status.probe.pypdfium2.clone().unwrap())],
            ),
        );
    }

    log(&app, t("log.done"));
    Ok(())
}

/// Turns a user-supplied markitdown location into a pip requirement.
fn resolve_markitdown_target(source: &str) -> String {
    let trimmed = source.trim();
    if !trimmed.is_empty() {
        let candidate = PathBuf::from(trimmed);
        // Accept either the package directory or the repository root.
        for probe in [
            candidate.clone(),
            candidate.join("packages").join("markitdown"),
        ] {
            if probe.join("pyproject.toml").is_file() {
                return format!("{}[{}]", probe.display(), MARKITDOWN_EXTRAS);
            }
        }
    }
    format!("markitdown[{MARKITDOWN_EXTRAS}]")
}
