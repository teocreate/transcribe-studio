//! Single source of truth for where the app keeps its data on disk.

use std::path::PathBuf;
use std::sync::OnceLock;

/// Tauri's resource directory, recorded once the app is set up.
static RESOURCE_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_resource_dir(dir: PathBuf) {
    let _ = RESOURCE_DIR.set(plain_path(dir));
}

/// Drops the `\\?\` verbatim prefix Windows APIs hand back. Child tools such
/// as Tesseract cannot open files through it.
fn plain_path(path: PathBuf) -> PathBuf {
    let text = path.to_string_lossy();
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::plain_path;
    use std::path::PathBuf;

    #[test]
    fn strips_verbatim_prefix() {
        assert_eq!(plain_path(PathBuf::from(r"\\?\C:\App\x")), PathBuf::from(r"C:\App\x"));
        assert_eq!(plain_path(PathBuf::from(r"\\?\UNC\srv\share")), PathBuf::from(r"\\srv\share"));
        assert_eq!(plain_path(PathBuf::from(r"C:\App")), PathBuf::from(r"C:\App"));
    }
}

/// `%LOCALAPPDATA%\TranscribeStudio`
pub fn data_root() -> PathBuf {
    let base = dirs::data_local_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join("TranscribeStudio")
}

/// Downloaded CTranslate2 Whisper models, one directory per model id.
pub fn models_dir() -> PathBuf {
    data_root().join("models")
}

/// The Python virtual environment the app provisions and owns.
pub fn venv_dir() -> PathBuf {
    data_root().join("venv")
}

/// Worker scripts unpacked from the binary at startup.
pub fn runtime_dir() -> PathBuf {
    data_root().join("runtime")
}

pub fn settings_file() -> PathBuf {
    data_root().join("settings.json")
}

pub fn log_dir() -> PathBuf {
    data_root().join("logs")
}

/// `python.exe` inside a venv directory.
pub fn venv_python(venv: &PathBuf) -> PathBuf {
    venv.join("Scripts").join("python.exe")
}

/// Folder of the bundled Tesseract OCR, if this build ships it.
///
/// The installer puts it under the resource directory; a portable copy finds
/// it only when a `tesseract` folder sits next to the executable.
pub fn tesseract_dir() -> Option<PathBuf> {
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("tesseract")));
    RESOURCE_DIR
        .get()
        .map(|dir| dir.join("tesseract"))
        .into_iter()
        .chain(beside_exe)
        .find(|dir| dir.join("tesseract.exe").is_file())
}

pub fn ensure_dirs() {
    for d in [models_dir(), runtime_dir(), log_dir()] {
        let _ = std::fs::create_dir_all(d);
    }
}
