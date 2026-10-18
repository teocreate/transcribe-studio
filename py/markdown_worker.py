"""Markdown conversion worker: a thin, streaming wrapper around markitdown.

Same newline-delimited JSON protocol as the transcription worker.
"""

import io
import json
import os
import subprocess
import sys
import time
import traceback

try:
    sys.stdout.reconfigure(encoding="utf-8", errors="replace", line_buffering=True)
    sys.stderr.reconfigure(encoding="utf-8", errors="replace")
except Exception:
    pass

# The UI shows a preview, not the whole document; sending a 40 MB string over
# the IPC bridge would stall the window for no benefit.
PREVIEW_LIMIT = 200_000


# Messages the user can see, in the interface language the job names.
MESSAGES = {
    "en": {
        "md_empty": "no text found, and recognition found nothing either.",
        "md_empty_no_ocr": "no text found. This looks like a scan, and scan recognition is unavailable: {why}.",
        "ocr_no_tesseract": "Tesseract is not bundled with this copy (use the installer)",
        "ocr_no_pdfium": "pypdfium2 is missing — open Settings → Environment and click “Set up environment”",
        "ocr_failed": "Tesseract failed: {err}",
        "no_job": "the worker was started without a job file",
        "md_import": "could not load markitdown: {exc}. Check the environment in Settings.",
    },
    "ru": {
        "md_empty": "текст не найден — распознавание тоже ничего не нашло.",
        "md_empty_no_ocr": "текст не найден. Похоже на скан, а распознавание сканов недоступно: {why}.",
        "ocr_no_tesseract": "в этой копии нет Tesseract (поставьте приложение через установщик)",
        "ocr_no_pdfium": "не хватает pypdfium2 — откройте «Настройки → Окружение» и нажмите «Настроить окружение»",
        "ocr_failed": "Tesseract завершился с ошибкой: {err}",
        "no_job": "воркер запущен без файла задания",
        "md_import": "не удалось загрузить markitdown: {exc}. Проверьте окружение в настройках.",
    },
    "es": {
        "md_empty": "no se encontró texto, y el reconocimiento tampoco encontró nada.",
        "md_empty_no_ocr": "no se encontró texto. Parece un escaneo y el reconocimiento no está disponible: {why}.",
        "ocr_no_tesseract": "esta copia no incluye Tesseract (usa el instalador)",
        "ocr_no_pdfium": "falta pypdfium2: abre Ajustes → Entorno y pulsa «Configurar entorno»",
        "ocr_failed": "Tesseract falló: {err}",
        "no_job": "el proceso se inició sin archivo de tarea",
        "md_import": "no se pudo cargar markitdown: {exc}. Revisa el entorno en Ajustes.",
    },
    "fr": {
        "md_empty": "aucun texte trouvé, et la reconnaissance n’a rien trouvé non plus.",
        "md_empty_no_ocr": "aucun texte trouvé. Cela ressemble à un scan, et la reconnaissance est indisponible : {why}.",
        "ocr_no_tesseract": "cette copie n’inclut pas Tesseract (utilisez l’installateur)",
        "ocr_no_pdfium": "pypdfium2 manque — ouvrez Réglages → Environnement et cliquez sur « Configurer l’environnement »",
        "ocr_failed": "Tesseract a échoué : {err}",
        "no_job": "le processus a été lancé sans fichier de tâche",
        "md_import": "impossible de charger markitdown : {exc}. Vérifiez l’environnement dans les Réglages.",
    },
}
_lang = "en"


def tr(key, **kwargs):
    table = MESSAGES.get(_lang) or MESSAGES["en"]
    return (table.get(key) or MESSAGES["en"][key]).format(**kwargs)


def emit(**payload):
    sys.stdout.write(json.dumps(payload, ensure_ascii=False) + "\n")
    sys.stdout.flush()


# Formats markitdown cannot read text from on its own.
IMAGE_EXTS = {".png", ".jpg", ".jpeg", ".tif", ".tiff", ".bmp", ".gif", ".webp"}
# A page with less text than this is treated as a scan.
MIN_PAGE_CHARS = 25
# Resolution scans are rendered at; Tesseract is tuned for about 300 dpi.
OCR_DPI = 300
# Keeps a console window from flashing up for every page.
CREATE_NO_WINDOW = 0x08000000


class Ocr:
    """Runs the bundled Tesseract on page images."""

    def __init__(self, cfg):
        self.exe = cfg["tesseract"]
        self.tessdata = cfg["tessdata"]
        self.langs = cfg["langs"]

    def image(self, img):
        buf = io.BytesIO()
        img.save(buf, format="PNG")
        proc = subprocess.run(
            [self.exe, "stdin", "stdout", "--tessdata-dir", self.tessdata, "-l", self.langs],
            input=buf.getvalue(),
            capture_output=True,
            creationflags=CREATE_NO_WINDOW,
        )
        if proc.returncode != 0:
            err = proc.stderr.decode("utf-8", "replace").strip().splitlines()
            raise RuntimeError(tr("ocr_failed", err=err[-1] if err else proc.returncode))
        return proc.stdout.decode("utf-8", "replace").strip()


def ocr_unavailable_reason(cfg):
    """Why scans cannot be read, or None when they can."""
    if not cfg:
        return tr("ocr_no_tesseract")
    try:
        import pypdfium2  # noqa: F401
    except Exception:
        return tr("ocr_no_pdfium")
    return None


def ocr_pdf(path, markdown, ocr, progress):
    """Fills in pages that have no text layer.

    A PDF where every page has text keeps markitdown's output, which knows
    about tables and headings. As soon as one page is a scan, the document is
    rebuilt page by page: the text layer where there is one, OCR elsewhere.
    """
    import pypdfium2 as pdfium

    pdf = pdfium.PdfDocument(path)
    try:
        texts = []
        for page in pdf:
            textpage = page.get_textpage()
            texts.append(textpage.get_text_range())
            textpage.close()
            page.close()
        scans = [i for i, text in enumerate(texts) if len(text.strip()) < MIN_PAGE_CHARS]
        if not scans:
            return markdown

        for done, i in enumerate(scans, 1):
            progress(done, len(scans))
            page = pdf[i]
            img = page.render(scale=OCR_DPI / 72, grayscale=True).to_pil()
            page.close()
            texts[i] = ocr.image(img)
        pages = [text.strip() for text in texts]
        return "\n\n---\n\n".join(text for text in pages if text)
    finally:
        pdf.close()


def ocr_image(path, markdown, ocr, progress):
    from PIL import Image

    progress(1, 1)
    with Image.open(path) as img:
        text = ocr.image(img.convert("L"))
    if markdown.strip() and text:
        return f"{markdown.rstrip()}\n\n{text}"
    return text or markdown


def unique_path(path):
    """Avoids silently overwriting an existing conversion."""
    if not os.path.exists(path):
        return path
    stem, ext = os.path.splitext(path)
    for n in range(2, 1000):
        candidate = f"{stem} ({n}){ext}"
        if not os.path.exists(candidate):
            return candidate
    return path


def main():
    if len(sys.argv) < 2:
        emit(t="fatal", msg=tr("no_job"))
        return 2

    with open(sys.argv[1], "r", encoding="utf-8") as fh:
        job = json.load(fh)

    global _lang
    _lang = job.get("ui_language") or "en"

    try:
        from markitdown import MarkItDown
    except Exception as exc:
        emit(t="fatal", msg=tr("md_import", exc=exc))
        return 2

    # Plugins are third-party code from site-packages; converting a document
    # should not quietly run them.
    converter = MarkItDown(enable_plugins=bool(job.get("enable_plugins")))

    ocr_cfg = job.get("ocr")
    ocr_missing = ocr_unavailable_reason(ocr_cfg)
    ocr = Ocr(ocr_cfg) if ocr_missing is None else None

    overwrite = bool(job.get("overwrite"))
    failures = 0
    files = job.get("files") or []
    emit(t="start", total=len(files))

    for index, path in enumerate(files):
        name = os.path.basename(path)
        emit(t="file_start", index=index, path=path, name=name)
        started = time.time()
        try:
            result = converter.convert(path)
            markdown = getattr(result, "markdown", None)
            if markdown is None:
                markdown = getattr(result, "text_content", "") or ""
            ext = os.path.splitext(path)[1].lower()
            if ocr and ext in (".pdf", *IMAGE_EXTS):
                def progress(page, pages, index=index):
                    emit(t="ocr", index=index, page=page, pages=pages)

                reader = ocr_pdf if ext == ".pdf" else ocr_image
                markdown = reader(path, markdown, ocr, progress)

            # Writing an empty file and calling it done hides the real problem
            # (usually a scan without a text layer).
            if not markdown.strip():
                if ocr is None and ext in (".pdf", *IMAGE_EXTS):
                    raise ValueError(tr("md_empty_no_ocr", why=ocr_missing))
                raise ValueError(tr("md_empty"))

            stem, _ = os.path.splitext(name)
            out_dir = job.get("output_dir") or os.path.dirname(path)
            os.makedirs(out_dir, exist_ok=True)
            target = os.path.join(out_dir, f"{stem}.md")
            if not overwrite:
                target = unique_path(target)

            with open(target, "w", encoding="utf-8") as fh:
                fh.write(markdown)

            emit(
                t="file_done",
                index=index,
                path=path,
                output=target,
                title=getattr(result, "title", None) or stem,
                chars=len(markdown),
                truncated=len(markdown) > PREVIEW_LIMIT,
                preview=markdown[:PREVIEW_LIMIT],
                elapsed=time.time() - started,
            )
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
