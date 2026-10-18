//! Persisted user preferences.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::paths;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Formats {
    pub txt: bool,
    pub srt: bool,
    pub vtt: bool,
    pub md: bool,
}

impl Default for Formats {
    fn default() -> Self {
        Self { txt: true, srt: false, vtt: false, md: true }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Interface language: `en`, `ru`, `es` or `fr`. Empty until the first
    /// launch resolves it from the installer choice or the system language.
    pub ui_language: String,
    /// The installer language (Windows LANGID) last applied, so a reinstall in
    /// another language takes effect once without overriding later choices
    /// made in the app.
    pub installer_langid: Option<u32>,
    /// `auto` | `light` | `dark`
    pub theme: String,
    /// Model id from the catalogue, e.g. `large-v3-turbo`.
    pub model: String,
    /// ISO code or `auto` for detection.
    pub language: String,
    /// CTranslate2 quantisation: `auto`, `int8`, `int8_float32`, `float32`, `float16`.
    pub compute_type: String,
    pub beam_size: u32,
    /// Voice-activity filter — trims silence, usually a clear win.
    pub vad: bool,
    /// 0 means "let CTranslate2 decide".
    pub cpu_threads: u32,
    /// `transcribe` keeps the source language, `translate` renders English.
    pub task: String,
    /// Optional vocabulary hint fed to the decoder.
    pub initial_prompt: String,
    pub prevent_sleep: bool,
    /// Empty means "write next to the source file".
    pub output_dir: String,
    pub formats: Formats,
    /// Additionally pipe the finished transcript through markitdown.
    pub run_markitdown: bool,
    /// Skip a file when its transcript already exists.
    pub skip_existing: bool,
    /// Where the standalone Markdown tab writes results. Empty = next to source.
    pub md_output_dir: String,
    /// Local markitdown checkout used when provisioning the environment.
    pub markitdown_source: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            ui_language: String::new(),
            installer_langid: None,
            theme: "auto".into(),
            model: "large-v3-turbo".into(),
            language: "ru".into(),
            compute_type: "auto".into(),
            beam_size: 5,
            vad: true,
            cpu_threads: 0,
            task: "transcribe".into(),
            initial_prompt: String::new(),
            prevent_sleep: true,
            output_dir: String::new(),
            formats: Formats::default(),
            run_markitdown: false,
            skip_existing: true,
            md_output_dir: String::new(),
            markitdown_source: String::new(),
        }
    }
}

impl Settings {
    /// Loads the saved settings; the flag is true when there were none yet,
    /// i.e. on the very first launch.
    pub fn load() -> (Self, bool) {
        match std::fs::read_to_string(paths::settings_file()) {
            Ok(text) => (serde_json::from_str(&text).unwrap_or_default(), false),
            Err(_) => (Self::default(), true),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let file = paths::settings_file();
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(file, serde_json::to_vec_pretty(self)?)?;
        Ok(())
    }
}

/// Process-wide settings, written through to disk on every change.
pub struct SettingsStore(Mutex<Settings>);

impl SettingsStore {
    pub fn new(initial: Settings) -> Self {
        Self(Mutex::new(initial))
    }

    pub fn get(&self) -> Settings {
        self.0.lock().unwrap().clone()
    }

    pub fn set(&self, next: Settings) -> anyhow::Result<()> {
        next.save()?;
        crate::i18n::set(&next.ui_language);
        *self.0.lock().unwrap() = next;
        Ok(())
    }
}
