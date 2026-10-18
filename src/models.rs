//! Whisper model catalogue and downloader.
//!
//! Models are CTranslate2 conversions pulled straight from Hugging Face into
//! `%LOCALAPPDATA%\TranscribeStudio\models\<id>`, which is then handed to
//! faster-whisper as a local path — so the app, not the Python layer, owns the
//! download and can report real progress.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use sha2::{Digest, Sha256};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;

use crate::i18n::{t, tf};
use crate::paths;

#[derive(Debug, Clone, Serialize)]
pub struct ModelSpec {
    pub id: &'static str,
    pub name: &'static str,
    pub repo: &'static str,
    /// Approximate on-disk size, shown in the UI before anything is fetched.
    pub size_mb: u32,
    pub params: &'static str,
    /// 1 (slowest) to 5 (fastest), relative, on CPU.
    pub speed: u8,
    /// 1 (roughest) to 5 (best) transcription quality.
    pub quality: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelState {
    #[serde(flatten)]
    pub spec: ModelSpec,
    pub installed: bool,
    /// Actual bytes on disk once installed.
    pub disk_bytes: u64,
    /// Bytes already fetched by an interrupted download, which the next
    /// attempt resumes from.
    pub partial_bytes: u64,
    pub path: String,
}

pub fn catalog() -> Vec<ModelSpec> {
    vec![
        ModelSpec {
            id: "tiny",
            name: "Tiny",
            repo: "Systran/faster-whisper-tiny",
            size_mb: 75,
            params: "39M",
            speed: 5,
            quality: 1,
        },
        ModelSpec {
            id: "base",
            name: "Base",
            repo: "Systran/faster-whisper-base",
            size_mb: 145,
            params: "74M",
            speed: 5,
            quality: 2,
        },
        ModelSpec {
            id: "small",
            name: "Small",
            repo: "Systran/faster-whisper-small",
            size_mb: 484,
            params: "244M",
            speed: 4,
            quality: 3,
        },
        ModelSpec {
            id: "medium",
            name: "Medium",
            repo: "Systran/faster-whisper-medium",
            size_mb: 1530,
            params: "769M",
            speed: 3,
            quality: 4,
        },
        ModelSpec {
            id: "large-v3-turbo",
            name: "Large v3 Turbo",
            repo: "deepdml/faster-whisper-large-v3-turbo-ct2",
            size_mb: 1620,
            params: "809M",
            speed: 4,
            quality: 5,
        },
        ModelSpec {
            id: "distil-large-v3",
            name: "Distil Large v3",
            repo: "Systran/faster-distil-whisper-large-v3",
            size_mb: 1510,
            params: "756M",
            speed: 5,
            quality: 4,
        },
        ModelSpec {
            id: "large-v2",
            name: "Large v2",
            repo: "Systran/faster-whisper-large-v2",
            size_mb: 3090,
            params: "1.55B",
            speed: 1,
            quality: 5,
        },
        ModelSpec {
            id: "large-v3",
            name: "Large v3",
            repo: "Systran/faster-whisper-large-v3",
            size_mb: 3090,
            params: "1.55B",
            speed: 1,
            quality: 5,
        },
    ]
}

pub fn spec(id: &str) -> Option<ModelSpec> {
    catalog().into_iter().find(|m| m.id == id)
}

pub fn model_path(id: &str) -> PathBuf {
    paths::models_dir().join(id)
}

/// A model counts as installed only once the weights file is in place.
pub fn is_installed(id: &str) -> bool {
    model_path(id).join("model.bin").is_file()
}

fn dir_size(path: &PathBuf) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub fn states() -> Vec<ModelState> {
    catalog()
        .into_iter()
        .map(|spec| {
            let path = model_path(spec.id);
            let installed = path.join("model.bin").is_file();
            ModelState {
                installed,
                disk_bytes: if installed { dir_size(&path) } else { 0 },
                partial_bytes: dir_size(&staging_path(spec.id)),
                path: path.to_string_lossy().into_owned(),
                spec,
            }
        })
        .collect()
}

/// Picks the model that suits this machine. The app runs Whisper on the CPU
/// only, so what matters is core count and memory, not the graphics card.
///
/// Large v3 Turbo keeps the full large encoder but only four decoder layers,
/// which makes it about as fast as Medium on a CPU while transcribing like
/// Large v3 — the right pick whenever the machine can hold it (~2 GB RAM in
/// int8) and has enough cores for the encoder. Below that, Small and Base
/// stay usable where a large model would crawl.
pub fn recommend(cpu_threads: usize, ram_gb: f64) -> &'static str {
    if ram_gb < 4.0 || cpu_threads <= 2 {
        "base"
    } else if ram_gb < 8.0 || cpu_threads < 6 {
        "small"
    } else {
        "large-v3-turbo"
    }
}

fn staging_path(id: &str) -> PathBuf {
    paths::models_dir().join(format!("{id}.partial"))
}

/// Removes staging directories that have been abandoned for a long time.
///
/// An interrupted download keeps its staging directory so the next attempt
/// can resume, but a model the user never came back for should not quietly
/// hold gigabytes forever.
pub fn sweep_partials() {
    const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 3600);
    let Ok(entries) = std::fs::read_dir(paths::models_dir()) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().ends_with(".partial") {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > MAX_AGE);
        if stale {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Deletes the installed model and any half-finished download of it.
pub fn delete(id: &str) -> Result<()> {
    for path in [model_path(id), staging_path(id)] {
        if path.exists() {
            std::fs::remove_dir_all(&path)
                .with_context(|| tf("err.delete_failed", &[("path", &path.display().to_string())]))?;
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct RepoInfo {
    siblings: Vec<Sibling>,
}

#[derive(Deserialize)]
struct Sibling {
    rfilename: String,
    size: Option<u64>,
    lfs: Option<Lfs>,
}

#[derive(Deserialize)]
struct Lfs {
    sha256: String,
}

#[derive(Serialize, Clone)]
pub struct DownloadProgress {
    pub id: String,
    pub file: String,
    pub done_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
    pub bytes_per_sec: f64,
    /// `download` while fetching, `verify` while checksumming the result.
    pub phase: &'static str,
}

/// Skips documentation and git metadata that would only waste bandwidth.
fn wanted(name: &str) -> bool {
    if name.starts_with('.') || name.contains("/.") {
        return false;
    }
    !name.to_ascii_lowercase().ends_with(".md")
}

fn sha256_file(path: &std::path::Path) -> Result<String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn file_len(path: &std::path::Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

pub async fn download(app: AppHandle, id: String, cancel: Arc<AtomicBool>) -> Result<()> {
    let spec = spec(&id).ok_or_else(|| anyhow!(tf("err.unknown_model", &[("id", &id)])))?;
    let target = model_path(&id);
    // Kept across interruptions: whatever is already in here is resumed.
    let staging = staging_path(&id);
    std::fs::create_dir_all(&staging)?;

    let client = reqwest::Client::builder()
        .user_agent("TranscribeStudio/1.0")
        .build()?;

    // `blobs=true` adds each file's size and, for LFS weights, its SHA-256 —
    // enough to resume exactly and to prove the result is intact.
    let info: RepoInfo = client
        .get(format!("https://huggingface.co/api/models/{}?blobs=true", spec.repo))
        .send()
        .await
        .context(t("err.hf_unreachable"))?
        .error_for_status()
        .context(t("err.hf_rejected"))?
        .json()
        .await
        .context(t("err.hf_parse"))?;

    let files: Vec<Sibling> = info
        .siblings
        .into_iter()
        .filter(|s| wanted(&s.rfilename))
        .collect();

    if files.is_empty() {
        bail!(tf("err.repo_empty", &[("repo", spec.repo)]));
    }

    let mut total: u64 = files.iter().filter_map(|f| f.size).sum();
    if total == 0 {
        total = u64::from(spec.size_mb) * 1024 * 1024;
    }

    let started = std::time::Instant::now();
    // Bytes already on disk count towards progress but not towards speed.
    let mut done: u64 = files
        .iter()
        .map(|f| {
            let have = file_len(&staging.join(&f.rfilename));
            f.size.map_or(0, |size| have.min(size))
        })
        .sum();
    let resumed_from = done;
    let mut last_emit = std::time::Instant::now();

    let emit = |file: &str, done: u64, total: u64, phase: &'static str| {
        let elapsed = started.elapsed().as_secs_f64();
        let fetched = done.saturating_sub(resumed_from);
        let _ = app.emit(
            "model:progress",
            DownloadProgress {
                id: id.clone(),
                file: file.to_string(),
                done_bytes: done,
                total_bytes: total,
                percent: if total > 0 {
                    (done as f64 / total as f64 * 100.0).min(100.0)
                } else {
                    0.0
                },
                bytes_per_sec: if elapsed > 0.5 { fetched as f64 / elapsed } else { 0.0 },
                phase,
            },
        );
    };

    emit("", done, total, "download");

    for file in &files {
        let name = &file.rfilename;
        if cancel.load(Ordering::SeqCst) {
            bail!(t("err.download_cancelled"));
        }

        let dest = staging.join(name);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut have = file_len(&dest);
        match file.size {
            Some(size) if have == size => continue,
            // Longer than the server's copy: the file changed upstream.
            Some(size) if have > size => {
                std::fs::remove_file(&dest).ok();
                done -= size;
                have = 0;
            }
            // Without a known size a partial file cannot be trusted.
            None if have > 0 => {
                std::fs::remove_file(&dest).ok();
                have = 0;
            }
            _ => {}
        }

        let url = format!("https://huggingface.co/{}/resolve/main/{}", spec.repo, name);
        let mut request = client.get(&url);
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let response = request
            .send()
            .await
            .with_context(|| tf("err.file_download", &[("file", name)]))?
            .error_for_status()
            .with_context(|| tf("err.file_rejected", &[("file", name)]))?;

        // A server that ignores the range sends the whole file again.
        let append = have > 0 && response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if have > 0 && !append {
            done -= have;
        }
        let mut out = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(&dest)
            .await?;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            if cancel.load(Ordering::SeqCst) {
                out.flush().await.ok();
                bail!(t("err.download_cancelled"));
            }
            let chunk = chunk.with_context(|| tf("err.connection_lost", &[("file", name)]))?;
            out.write_all(&chunk).await?;
            done += chunk.len() as u64;

            // ~15 fps is plenty for a progress bar and keeps the IPC quiet.
            if last_emit.elapsed().as_millis() >= 66 {
                last_emit = std::time::Instant::now();
                emit(name, done, total.max(done), "download");
            }
        }
        out.flush().await?;
    }

    // Integrity pass over everything, including files kept from an earlier
    // attempt: a resumed download is only as good as the bytes it resumed.
    let final_total = total.max(done);
    for file in &files {
        let name = &file.rfilename;
        let dest = staging.join(name);
        let have = file_len(&dest);
        if let Some(size) = file.size {
            if have != size {
                std::fs::remove_file(&dest).ok();
                bail!(tf(
                    "err.size_mismatch",
                    &[("file", name), ("got", &have.to_string()), ("want", &size.to_string())]
                ));
            }
        }
        if let Some(lfs) = &file.lfs {
            emit(name, final_total, final_total, "verify");
            let path = dest.clone();
            let actual = tokio::task::spawn_blocking(move || sha256_file(&path)).await??;
            if !actual.eq_ignore_ascii_case(&lfs.sha256) {
                std::fs::remove_file(&dest).ok();
                bail!(tf("err.hash_mismatch", &[("file", name)]));
            }
        }
    }

    emit("", final_total, final_total, "download");

    if !staging.join("model.bin").is_file() {
        std::fs::remove_dir_all(&staging).ok();
        bail!(t("err.no_model_bin"));
    }

    // Swap in the finished copy only at the very end.
    if target.exists() {
        std::fs::remove_dir_all(&target).ok();
    }
    std::fs::rename(&staging, &target).context(t("err.move_model"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::recommend;

    #[test]
    fn recommendation_follows_hardware() {
        assert_eq!(recommend(12, 13.7), "large-v3-turbo");
        assert_eq!(recommend(4, 16.0), "small");
        assert_eq!(recommend(8, 6.0), "small");
        assert_eq!(recommend(2, 8.0), "base");
        assert_eq!(recommend(8, 3.5), "base");
    }
}
