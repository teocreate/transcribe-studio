# Transcribe Studio

A Windows desktop app for **local** speech-to-text and document-to-Markdown
conversion. Audio and video are transcribed on your own machine with
[Whisper](https://github.com/openai/whisper) models; documents, scans and
images become Markdown through
[MarkItDown](https://github.com/microsoft/markitdown) and
[Tesseract OCR](https://github.com/tesseract-ocr/tesseract). Nothing is sent
to the cloud.

**[Русская версия →](README.ru.md)**

> Transcribe Studio is an independent project. It is a desktop front end for
> open-source work by OpenAI, SYSTRAN, Microsoft and the Tesseract
> contributors, and is not affiliated with or endorsed by any of them. See
> [Built on](#built-on) below.

## Download

Grab the latest build from [Releases](../../releases):

- **`Transcribe-Studio_<version>_x64-setup.exe`** — installer. Installs for
  all users into `C:\Program Files\Transcribe Studio` (asks for administrator
  rights) and includes OCR.
- **`Transcribe-Studio_<version>_x64-portable.zip`** — no installation; unzip
  and run. Keep the `tesseract` folder next to the exe for scan recognition.

On first launch the app offers **Install everything**: it creates its own
Python environment and downloads the Whisper model recommended for your
hardware. That needs an internet connection and Python 3.11 or newer installed
(from [python.org](https://www.python.org/downloads/)); it usually takes
5–15 minutes. Everything it downloads lives in
`%LOCALAPPDATA%\TranscribeStudio`.

## Features

**Transcription**

- Queue of audio/video files (m4a, mp3, wav, flac, mp4, mkv and anything else
  FFmpeg reads), by drag and drop or file picker.
- Model, language, task (transcribe or translate to English), beam size, CPU
  threads, compute type, silence filter (VAD) and a decoder hint for names
  and jargon.
- Live progress: percentage, position, speed relative to real time, time
  left, and the recognized text as it arrives.
- Output as TXT, SRT, VTT and Markdown (with metadata and timestamps).
- Keeps the PC from sleeping during long runs.

**Markdown**

- Converts PDF, DOCX, XLSX, PPTX, HTML, CSV, EPUB, images, archives and more.
- Scanned PDFs and images are read with the bundled Tesseract. In a PDF only
  pages without a text layer are recognized; the rest keep MarkItDown's
  output.
- Preview with rendered/source views; in a batch, click any row to open its
  result.

**Settings**

- Model manager for eight Whisper variants with size, speed and quality,
  progress, cancel, **resume** of interrupted downloads and SHA-256
  verification.
- The recommended model is picked from your CPU threads and RAM — Whisper
  runs on the CPU here, no GPU needed.
- Interface in **English, Русский, Español, Français**. The installer offers
  your system language (English if it isn't one of these).
- Light, dark or system appearance.

## Built on

| Project | By | Used for | License |
|---|---|---|---|
| [Whisper](https://github.com/openai/whisper) | OpenAI | Speech recognition models | MIT |
| [faster-whisper](https://github.com/SYSTRAN/faster-whisper) | SYSTRAN | Runs Whisper efficiently on the CPU via [CTranslate2](https://github.com/OpenNMT/CTranslate2); model conversions on [Hugging Face](https://huggingface.co/Systran) | MIT |
| [MarkItDown](https://github.com/microsoft/markitdown) | Microsoft | Converting documents to Markdown | MIT |
| [Tesseract OCR](https://github.com/tesseract-ocr/tesseract) | Tesseract contributors ([Windows build](https://github.com/UB-Mannheim/tesseract) by UB Mannheim) | Recognizing scans and images | Apache 2.0 |
| [Tauri](https://tauri.app) | Tauri contributors | Native window and app shell | MIT / Apache 2.0 |

Whisper models and Python packages are downloaded from their original sources
at setup time. The bundled Tesseract binaries and language data keep their own
license — see [`resources/tesseract/README.txt`](resources/tesseract/README.txt).

## How it works

```
ui/          HTML/CSS/JS window, compiled into the binary
  ↕ Tauri IPC
src/*.rs     Rust: window, settings, model downloads, processes, sleep lock
  ↕ child processes speaking newline-delimited JSON
py/*.py      two stateless Python workers: faster-whisper and MarkItDown (+ Tesseract)
```

Rust owns all state and every process; Python only does what has to be
Python. Interface strings live in `ui/locales/*.js`; strings born in Rust are
in `src/i18n.rs`.

## Building

Needs Rust (MSVC toolchain), the [Tauri CLI](https://tauri.app/start/) and
WebView2 (built into Windows 11).

```powershell
cargo build --release      # just the exe
cargo tauri build          # exe + NSIS installer
```

Checks:

```powershell
cargo test
node tests/markdown.test.mjs    # Markdown preview renderer
node tests/locales.test.mjs     # every language defines the same keys
```

`tools/*.ps1` drive the real window with synthetic input and save screenshots
to `%TEMP%` — see [README.ru.md](README.ru.md#проверки) for the list.

## Contributing

The code is open for improvements and suggestions — issues and pull requests
are welcome, whether it's a bug report, a new interface language, a feature
idea or a fix. For a new language: add `ui/locales/<code>.js`, a column in
`src/i18n.rs`, the worker messages in `py/*.py`, and run
`node tests/locales.test.mjs`.

## License

[MIT](LICENSE) for Transcribe Studio's own code. Third-party components keep
their own licenses, listed above.
