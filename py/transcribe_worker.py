"""Transcription worker for Transcribe Studio.

Reads a job description from a JSON file and streams newline-delimited JSON
events on stdout so the Rust side can render real progress. Every event has a
``t`` field naming its kind; anything unparseable on stdout is a bug, so the
worker never prints free-form text.
"""

import datetime as _dt
import json
import os
import sys
import time
import traceback

# stdout carries the event protocol and must stay UTF-8 and line-buffered even
# when the parent process is not a console.
try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace", line_buffering=True)
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass


# Messages the user can see, in the interface language the job names.
MESSAGES = {
    "en": {
        "no_job": "the worker was started without a job file",
        "skip_exists": "output already exists",
        "fw_import": "could not load faster-whisper: {exc}. Check the environment in Settings.",
        "model_dir": "model folder not found: {path}",
        "fallback": "{compute} is unavailable ({exc}); switching to int8",
        "model_load": "could not load the model: {exc}",
    },
    "ru": {
        "no_job": "воркер запущен без файла задания",
        "skip_exists": "результат уже существует",
        "fw_import": "не удалось загрузить faster-whisper: {exc}. Проверьте окружение в настройках.",
        "model_dir": "каталог модели не найден: {path}",
        "fallback": "{compute} недоступен ({exc}); переключаюсь на int8",
        "model_load": "не удалось загрузить модель: {exc}",
    },
    "es": {
        "no_job": "el proceso se inició sin archivo de tarea",
        "skip_exists": "el resultado ya existe",
        "fw_import": "no se pudo cargar faster-whisper: {exc}. Revisa el entorno en Ajustes.",
        "model_dir": "no se encontró la carpeta del modelo: {path}",
        "fallback": "{compute} no está disponible ({exc}); cambiando a int8",
        "model_load": "no se pudo cargar el modelo: {exc}",
    },
    "fr": {
        "no_job": "le processus a été lancé sans fichier de tâche",
        "skip_exists": "le résultat existe déjà",
        "fw_import": "impossible de charger faster-whisper : {exc}. Vérifiez l’environnement dans les Réglages.",
        "model_dir": "dossier du modèle introuvable : {path}",
        "fallback": "{compute} indisponible ({exc}) ; passage en int8",
        "model_load": "impossible de charger le modèle : {exc}",
    },
}
_lang = "en"


def tr(key, **kwargs):
    table = MESSAGES.get(_lang) or MESSAGES["en"]
    return (table.get(key) or MESSAGES["en"][key]).format(**kwargs)


def emit(**payload):
    sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
    sys.stdout.flush()


def log(msg):
    emit(t="log", msg=str(msg))


def _patch_av():
    """Tolerate PyAV builds that predate the ``metadata_errors`` argument.

    faster-whisper passes it unconditionally; older PyAV raises TypeError.
    Retrying without the argument is harmless on every version.
    """
    try:
        import av
    except Exception:
        return

    original = av.open

    def opener(*args, **kwargs):
        try:
            return original(*args, **kwargs)
        except TypeError as exc:
            if "metadata_errors" not in str(exc):
                raise
            kwargs.pop("metadata_errors", None)
            return original(*args, **kwargs)

    av.open = opener


def fmt_hms(seconds):
    seconds = max(0, int(seconds or 0))
    h, rem = divmod(seconds, 3600)
    m, s = divmod(rem, 60)
    return f"{h}:{m:02d}:{s:02d}" if h else f"{m}:{s:02d}"


def srt_time(seconds):
    seconds = max(0.0, float(seconds or 0.0))
    ms = int(round(seconds * 1000))
    h, ms = divmod(ms, 3_600_000)
    m, ms = divmod(ms, 60_000)
    s, ms = divmod(ms, 1000)
    return f"{h:02d}:{m:02d}:{s:02d},{ms:03d}"


def vtt_time(seconds):
    return srt_time(seconds).replace(",", ".")


def write_txt(path, segments):
    with open(path, "w", encoding="utf-8") as fh:
        for seg in segments:
            fh.write(seg["text"] + "\n")


def write_srt(path, segments):
    with open(path, "w", encoding="utf-8") as fh:
        for i, seg in enumerate(segments, 1):
            fh.write(f"{i}\n")
            fh.write(f"{srt_time(seg['start'])} --> {srt_time(seg['end'])}\n")
            fh.write(seg["text"] + "\n\n")


def write_vtt(path, segments):
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("WEBVTT\n\n")
        for seg in segments:
            fh.write(f"{vtt_time(seg['start'])} --> {vtt_time(seg['end'])}\n")
            fh.write(seg["text"] + "\n\n")


# A new timestamped block starts after this much speech, or after a pause this
# long — long enough to stay readable, short enough to navigate.
_MD_BLOCK_SECONDS = 45.0
_MD_PAUSE_SECONDS = 2.5


def write_md(path, segments, meta):
    """Writes a transcript that is actually pleasant to read.

    Plain text piped through a converter stays a wall of text, so the Markdown
    output is built here: front matter with the provenance of the run, then
    paragraphs anchored by timestamps.
    """
    blocks = []
    current = None
    for seg in segments:
        gap = seg["start"] - current["end"] if current else 0.0
        too_long = current and (seg["end"] - current["start"]) > _MD_BLOCK_SECONDS
        if current is None or too_long or gap > _MD_PAUSE_SECONDS:
            current = {"start": seg["start"], "end": seg["end"], "parts": [seg["text"]]}
            blocks.append(current)
        else:
            current["parts"].append(seg["text"])
            current["end"] = seg["end"]

    with open(path, "w", encoding="utf-8") as fh:
        fh.write("---\n")
        fh.write(f"source: {json.dumps(meta['source_name'], ensure_ascii=False)}\n")
        fh.write(f"duration: {fmt_hms(meta['duration'])}\n")
        fh.write(f"model: {meta['model']}\n")
        fh.write(f"language: {meta['language']}\n")
        fh.write(f"transcribed: {meta['timestamp']}\n")
        fh.write("---\n\n")
        fh.write(f"# {meta['title']}\n\n")
        for block in blocks:
            text = " ".join(part for part in block["parts"] if part).strip()
            if not text:
                continue
            fh.write(f"**[{fmt_hms(block['start'])}]** {text}\n\n")


def transcribe_one(model, index, path, job, model_label):
    name = os.path.basename(path)
    stem, _ = os.path.splitext(name)
    out_dir = job.get("output_dir") or os.path.dirname(path)
    os.makedirs(out_dir, exist_ok=True)

    formats = job.get("formats") or {}
    enabled = [key for key in ("txt", "srt", "vtt", "md") if formats.get(key)]
    if not enabled:
        enabled = ["txt"]
    targets = {key: os.path.join(out_dir, f"{stem}.{key}") for key in enabled}

    if job.get("skip_existing") and all(os.path.exists(p) for p in targets.values()):
        emit(t="file_skip", index=index, path=path, reason=tr("skip_exists"))
        return list(targets.values())

    emit(t="file_start", index=index, path=path, name=name)
    started = time.time()

    kwargs = {
        "task": job.get("task") or "transcribe",
        "beam_size": int(job.get("beam_size") or 5),
        "vad_filter": bool(job.get("vad", True)),
        # Repetition loops on long recordings are far more damaging than the
        # small coherence gain from conditioning on previous output.
        "condition_on_previous_text": False,
    }
    language = job.get("language")
    if language and language != "auto":
        kwargs["language"] = language
    prompt = (job.get("initial_prompt") or "").strip()
    if prompt:
        kwargs["initial_prompt"] = prompt

    segment_iter, info = model.transcribe(path, **kwargs)

    duration = float(getattr(info, "duration", 0.0) or 0.0)
    detected = getattr(info, "language", None) or language or "?"
    probability = float(getattr(info, "language_probability", 0.0) or 0.0)
    emit(
        t="meta",
        index=index,
        duration=duration,
        language=detected,
        language_probability=probability,
    )

    segments = []
    for seg in segment_iter:
        text = (seg.text or "").strip()
        record = {"start": float(seg.start), "end": float(seg.end), "text": text}
        segments.append(record)
        emit(
            t="seg",
            index=index,
            start=record["start"],
            end=record["end"],
            text=text,
            duration=duration,
        )

    meta = {
        "source_name": name,
        "title": stem,
        "duration": duration,
        "model": model_label,
        "language": f"{detected} ({probability:.2f})" if probability else str(detected),
        "timestamp": _dt.datetime.now().strftime("%Y-%m-%d %H:%M"),
    }

    writers = {"txt": write_txt, "srt": write_srt, "vtt": write_vtt}
    written = []
    for key, target in targets.items():
        if key == "md":
            write_md(target, segments, meta)
        else:
            writers[key](target, segments)
        written.append(target)

    emit(
        t="file_done",
        index=index,
        path=path,
        outputs=written,
        elapsed=time.time() - started,
        segments=len(segments),
        duration=duration,
    )
    return written


def main():
    if len(sys.argv) < 2:
        emit(t="fatal", msg=tr("no_job"))
        return 2

    with open(sys.argv[1], "r", encoding="utf-8") as fh:
        job = json.load(fh)

    global _lang
    _lang = job.get("ui_language") or "en"

    _patch_av()

    try:
        from faster_whisper import WhisperModel
    except Exception as exc:
        emit(t="fatal", msg=tr("fw_import", exc=exc))
        return 2

    model_path = job["model_path"]
    if not os.path.isdir(model_path):
        emit(t="fatal", msg=tr("model_dir", path=model_path))
        return 2

    threads = int(job.get("cpu_threads") or 0)
    compute_type = job.get("compute_type") or "auto"
    device = job.get("device") or "cpu"

    emit(
        t="env",
        device=device,
        compute_type=compute_type,
        cpu_threads=threads or (os.cpu_count() or 0),
        model=job.get("model_id") or os.path.basename(model_path),
    )

    load_kwargs = {"device": device, "compute_type": compute_type}
    if threads > 0:
        load_kwargs["cpu_threads"] = threads

    try:
        model = WhisperModel(model_path, **load_kwargs)
    except Exception as exc:
        # `auto` resolves to a quantisation the CPU may not support; int8 is
        # available everywhere and is the right fallback rather than failing.
        if compute_type != "int8":
            log(tr("fallback", compute=compute_type, exc=exc))
            load_kwargs["compute_type"] = "int8"
            try:
                model = WhisperModel(model_path, **load_kwargs)
            except Exception as exc2:
                emit(t="fatal", msg=tr("model_load", exc=exc2))
                return 2
        else:
            emit(t="fatal", msg=tr("model_load", exc=exc))
            return 2

    model_label = job.get("model_id") or os.path.basename(model_path)
    failures = 0
    for index, path in enumerate(job.get("files") or []):
        try:
            transcribe_one(model, index, path, job, model_label)
        except KeyboardInterrupt:
            raise
        except Exception as exc:
            failures += 1
            emit(
                t="file_error",
                index=index,
                path=path,
                msg=str(exc),
                detail=traceback.format_exc(limit=3),
            )

    emit(t="all_done", failures=failures)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except KeyboardInterrupt:
        emit(t="cancelled")
        sys.exit(130)
