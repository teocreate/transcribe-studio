//! Interface language: which one to use, and the handful of strings the Rust
//! side produces itself (errors, dialog filters, setup log).
//!
//! Everything the window shows lives in `ui/locales/*.js`; only text that is
//! born here — an error bubbling out of a command, a file-dialog filter —
//! needs a translation on this side.

use std::sync::RwLock;

/// Languages the interface ships in. The first one is the fallback.
pub const SUPPORTED: &[&str] = &["en", "ru", "es", "fr"];

static CURRENT: RwLock<&'static str> = RwLock::new("en");

/// Normalises any language tag (`ru-RU`, `fr`, `ES`) to a supported code.
pub fn normalize(tag: &str) -> Option<&'static str> {
    let primary = tag.split(['-', '_']).next()?.to_ascii_lowercase();
    SUPPORTED.iter().copied().find(|code| *code == primary)
}

pub fn set(lang: &str) {
    *CURRENT.write().unwrap() = normalize(lang).unwrap_or(SUPPORTED[0]);
}

pub fn current() -> &'static str {
    *CURRENT.read().unwrap()
}

/// Maps a Windows LANGID to a supported code by its primary language.
fn from_langid(langid: u32) -> Option<&'static str> {
    match langid & 0x3ff {
        0x09 => Some("en"),
        0x19 => Some("ru"),
        0x0a => Some("es"),
        0x0c => Some("fr"),
        _ => None,
    }
}

/// The LANGID the NSIS installer stored for the language picked in its
/// language dialog. Tauri's template keeps it under
/// `HKCU\Software\<publisher>\<product>\Installer Language`; the portable
/// build never writes it.
#[cfg(windows)]
pub fn installer_langid() -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};

    let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
    let key = wide(r"Software\fyodor\Transcribe Studio");
    let value = wide("Installer Language");
    let mut buf = [0u16; 16];
    let mut size = (buf.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len]).trim().parse().ok()
}

#[cfg(not(windows))]
pub fn installer_langid() -> Option<u32> {
    None
}

pub fn installer_language() -> Option<&'static str> {
    installer_langid().and_then(from_langid)
}

/// The Windows display language, if the interface is available in it.
#[cfg(windows)]
pub fn system_language() -> Option<&'static str> {
    use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
    from_langid(u32::from(unsafe { GetUserDefaultUILanguage() }))
}

#[cfg(not(windows))]
pub fn system_language() -> Option<&'static str> {
    None
}

/// Language for a user who has not chosen one in the app: the installer's
/// choice, then the system language, then English.
pub fn detect() -> &'static str {
    installer_language()
        .or_else(system_language)
        .unwrap_or(SUPPORTED[0])
}

/// Looks up a backend string in the current language.
pub fn t(key: &str) -> &'static str {
    let lang = current();
    let row = STRINGS
        .iter()
        .find(|row| row.0 == key)
        .unwrap_or_else(|| panic!("missing backend string: {key}"));
    match lang {
        "ru" => row.2,
        "es" => row.3,
        "fr" => row.4,
        _ => row.1,
    }
}

/// `t` with `{name}`-style placeholders filled in.
pub fn tf(key: &str, args: &[(&str, &str)]) -> String {
    let mut out = t(key).to_string();
    for (name, value) in args {
        out = out.replace(&format!("{{{name}}}"), value);
    }
    out
}

/// key, en, ru, es, fr
const STRINGS: &[(&str, &str, &str, &str, &str)] = &[
    ("filter.all", "All files", "Все файлы", "Todos los archivos", "Tous les fichiers"),
    ("filter.media", "Audio and video", "Аудио и видео", "Audio y vídeo", "Audio et vidéo"),
    ("filter.documents", "Documents", "Документы", "Documentos", "Documents"),
    (
        "err.model_missing",
        "Model “{name}” is not downloaded. Open Settings → Models and download it.",
        "Модель «{name}» не загружена. Откройте «Настройки → Модели» и скачайте её.",
        "El modelo «{name}» no está descargado. Abre Ajustes → Modelos y descárgalo.",
        "Le modèle « {name} » n’est pas téléchargé. Ouvrez Réglages → Modèles pour le télécharger.",
    ),
    ("err.download_cancelled", "Download cancelled", "Загрузка отменена", "Descarga cancelada", "Téléchargement annulé"),
    ("err.delete_failed", "Could not delete {path}", "Не удалось удалить {path}", "No se pudo eliminar {path}", "Impossible de supprimer {path}"),
    ("err.unknown_model", "Unknown model: {id}", "Неизвестная модель: {id}", "Modelo desconocido: {id}", "Modèle inconnu : {id}"),
    ("err.hf_unreachable", "Could not reach Hugging Face", "Не удалось связаться с Hugging Face", "No se pudo conectar con Hugging Face", "Impossible de joindre Hugging Face"),
    ("err.hf_rejected", "Hugging Face returned an error for this repository", "Hugging Face вернул ошибку для этого репозитория", "Hugging Face devolvió un error para este repositorio", "Hugging Face a renvoyé une erreur pour ce dépôt"),
    ("err.hf_parse", "Could not read the model’s file list", "Не удалось разобрать список файлов модели", "No se pudo leer la lista de archivos del modelo", "Impossible de lire la liste des fichiers du modèle"),
    ("err.repo_empty", "Repository {repo} has nothing to download", "В репозитории {repo} нет файлов для скачивания", "El repositorio {repo} no tiene archivos para descargar", "Le dépôt {repo} ne contient rien à télécharger"),
    ("err.file_download", "Could not download {file}", "Не удалось скачать {file}", "No se pudo descargar {file}", "Impossible de télécharger {file}"),
    ("err.file_rejected", "The server rejected {file}", "Сервер отклонил запрос {file}", "El servidor rechazó {file}", "Le serveur a refusé {file}"),
    ("err.connection_lost", "Connection lost while downloading {file}", "Обрыв соединения на {file}", "Se perdió la conexión al descargar {file}", "Connexion perdue pendant le téléchargement de {file}"),
    (
        "err.no_model_bin",
        "The download has no model.bin — this repository does not look like a CTranslate2 model",
        "Среди скачанных файлов нет model.bin — репозиторий не похож на CTranslate2-модель",
        "La descarga no contiene model.bin: este repositorio no parece un modelo CTranslate2",
        "Le téléchargement ne contient pas model.bin : ce dépôt ne ressemble pas à un modèle CTranslate2",
    ),
    ("err.move_model", "Could not move the model into place", "Не удалось переместить модель на место", "No se pudo mover el modelo a su ubicación", "Impossible de mettre le modèle en place"),
    (
        "err.size_mismatch",
        "{file} is incomplete: {got} of {want} bytes. Try downloading again.",
        "{file} скачан не полностью: {got} из {want} байт. Попробуйте скачать ещё раз.",
        "{file} está incompleto: {got} de {want} bytes. Vuelve a intentar la descarga.",
        "{file} est incomplet : {got} octets sur {want}. Relancez le téléchargement.",
    ),
    (
        "err.hash_mismatch",
        "{file} is corrupted (checksum mismatch). Try downloading again.",
        "{file} повреждён (контрольная сумма не совпала). Попробуйте скачать ещё раз.",
        "{file} está dañado (la suma de verificación no coincide). Vuelve a intentar la descarga.",
        "{file} est corrompu (somme de contrôle incorrecte). Relancez le téléchargement.",
    ),
    (
        "err.env_missing",
        "No Python environment found. Open Settings → Environment and click “Set up environment”.",
        "Python-окружение не найдено. Откройте «Настройки → Окружение» и нажмите «Настроить окружение».",
        "No se encontró el entorno de Python. Abre Ajustes → Entorno y pulsa «Configurar entorno».",
        "Aucun environnement Python trouvé. Ouvrez Réglages → Environnement et cliquez sur « Configurer l’environnement ».",
    ),
    ("err.spawn", "Could not start: {what}", "Не удалось запустить: {what}", "No se pudo iniciar: {what}", "Impossible de lancer : {what}"),
    ("err.step_failed", "Step failed ({what})", "Шаг завершился с ошибкой ({what})", "El paso falló ({what})", "L’étape a échoué ({what})"),
    (
        "err.no_python",
        "Python not found. Install Python 3.12 from python.org and try again.",
        "Python не найден. Установите Python 3.12 с python.org и повторите.",
        "No se encontró Python. Instala Python 3.12 desde python.org y vuelve a intentarlo.",
        "Python introuvable. Installez Python 3.12 depuis python.org puis réessayez.",
    ),
    (
        "err.venv_dead",
        "The environment was created, but its interpreter does not respond",
        "Окружение создано, но интерпретатор в нём не отвечает",
        "El entorno se creó, pero su intérprete no responde",
        "L’environnement a été créé, mais son interpréteur ne répond pas",
    ),
    ("err.busy", "A task is already running", "Задача уже выполняется", "Ya hay una tarea en curso", "Une tâche est déjà en cours"),
    ("err.worker_exit", "The worker exited with code {code}", "Воркер завершился с кодом {code}", "El proceso terminó con el código {code}", "Le processus s’est arrêté avec le code {code}"),
    ("err.no_files", "No files selected", "Не выбрано ни одного файла", "No hay archivos seleccionados", "Aucun fichier sélectionné"),
    ("log.markitdown_files", "markitdown: {n} file(s)", "markitdown: файлов — {n}", "markitdown: {n} archivo(s)", "markitdown : {n} fichier(s)"),
    ("log.markitdown_failed", "markitdown failed: {err}", "markitdown не справился: {err}", "markitdown falló: {err}", "markitdown a échoué : {err}"),
    ("log.creating_venv", "Creating a virtual environment…", "Создаю виртуальное окружение…", "Creando un entorno virtual…", "Création d’un environnement virtuel…"),
    ("log.env", "Environment: {path} (Python {version})", "Окружение: {path} (Python {version})", "Entorno: {path} (Python {version})", "Environnement : {path} (Python {version})"),
    ("log.installing", "Installing {what}…", "Устанавливаю {what}…", "Instalando {what}…", "Installation de {what}…"),
    ("log.installing_from", "Installing markitdown from {source}…", "Устанавливаю markitdown из {source}…", "Instalando markitdown desde {source}…", "Installation de markitdown depuis {source}…"),
    ("log.already", "{what} {version} is already installed", "{what} {version} уже установлен", "{what} {version} ya está instalado", "{what} {version} est déjà installé"),
    ("log.done", "Done.", "Готово.", "Listo.", "Terminé."),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize("ru-RU"), Some("ru"));
        assert_eq!(normalize("FR"), Some("fr"));
        assert_eq!(normalize("de-DE"), None);
    }

    #[test]
    fn maps_installer_langids() {
        assert_eq!(from_langid(1049), Some("ru"));
        assert_eq!(from_langid(1033), Some("en"));
        assert_eq!(from_langid(1034), Some("es"));
        assert_eq!(from_langid(3082), Some("es"));
        assert_eq!(from_langid(1036), Some("fr"));
        assert_eq!(from_langid(1031), None);
    }

    #[test]
    fn every_key_is_unique() {
        let mut keys: Vec<_> = STRINGS.iter().map(|r| r.0).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), STRINGS.len());
    }
}
