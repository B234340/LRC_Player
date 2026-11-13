"""Audio side of lrc-align: decoding, vocal separation (Demucs), forced alignment (stable-ts).

Imports of the heavy dependencies happen inside the functions so `--help` and the pure
tests work without them.
"""

from __future__ import annotations

import shutil
import subprocess
from typing import Callable

from .core import Line, Lrc, Word, estimate_line, words_to_chars

ALIGN_RATE = 16_000
# Audio kept around each line (ms) so the first / last syllable is not cut.
PAD_BEFORE_MS = 300
PAD_AFTER_MS = 300


class MissingDependency(RuntimeError):
    pass


def _require(module: str, extra: str = "lrc-align[ml]"):
    try:
        return __import__(module)
    except ImportError as e:  # pragma: no cover - depends on the environment
        raise MissingDependency(f"{module} is not installed (pip install {extra})") from e


def load_audio(path: str):
    """Returns (float32 array [channels, samples], sample rate)."""
    np = _require("numpy")
    try:
        sf = _require("soundfile")
        data, sr = sf.read(path, dtype="float32", always_2d=True)
        return np.ascontiguousarray(data.T), sr
    except MissingDependency:
        raise
    except Exception:
        pass
    # Formats libsndfile cannot read (m4a/aac, ...): decode with ffmpeg.
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        raise RuntimeError(f"cannot decode {path}: install ffmpeg (needed for this format)")
    sr = 44_100
    raw = subprocess.run(
        [ffmpeg, "-nostdin", "-v", "error", "-i", path, "-f", "f32le", "-ac", "2", "-ar", str(sr), "-"],
        check=True,
        capture_output=True,
    ).stdout
    data = np.frombuffer(raw, dtype=np.float32).reshape(-1, 2).T
    return np.ascontiguousarray(data), sr


def separate_vocals(wav, sr: int, device: str = "cpu", log: Callable[[str], None] = print):
    """Vocals (mono float32 array) and their sample rate, using Demucs `htdemucs`."""
    torch = _require("torch")
    _require("demucs")
    import julius
    from demucs.apply import apply_model
    from demucs.pretrained import get_model

    model = get_model("htdemucs")
    model.eval()
    x = torch.from_numpy(wav)
    if x.shape[0] == 1:
        x = x.repeat(2, 1)
    elif x.shape[0] > 2:
        x = x[:2]
    if sr != model.samplerate:
        x = julius.resample_frac(x, sr, model.samplerate)
    ref = x.mean(0)
    mean, std = ref.mean(), ref.std().clamp_min(1e-8)
    log("separating vocals (Demucs htdemucs)...")
    with torch.no_grad():
        sources = apply_model(model, ((x - mean) / std)[None], device=device, split=True, overlap=0.25, progress=False)[0]
    sources = sources * std + mean
    vocals = sources[model.sources.index("vocals")].mean(0)
    return vocals.cpu().numpy(), model.samplerate


def to_align_rate(mono, sr: int):
    torch = _require("torch")
    import julius

    x = torch.from_numpy(mono)
    if sr != ALIGN_RATE:
        x = julius.resample_frac(x, sr, ALIGN_RATE)
    return x.numpy().astype("float32")


def align_lines(
    lrc: Lrc,
    audio16k,
    japanese: bool,
    model_name: str = "small",
    language: str | None = None,
    device: str = "cpu",
    log: Callable[[str], None] = print,
) -> tuple[int, int]:
    """Aligns every non-blank line to its window of `audio16k`; lines that cannot be aligned get
    the estimate. Returns (aligned, estimated) line counts."""
    stable_whisper = _require("stable_whisper", "stable-ts")
    log(f"loading Whisper model '{model_name}' ({device})...")
    model = stable_whisper.load_model(model_name, device=device)
    total_ms = len(audio16k) * 1000 // ALIGN_RATE
    aligned = estimated = 0
    todo = [l for l in lrc.lines if l.text.strip()]
    for k, line in enumerate(todo, 1):
        s = max(0, line.start_ms - PAD_BEFORE_MS)
        e = min(total_ms, line.end_ms + PAD_AFTER_MS)
        ok = False
        if e - s >= 200:
            seg = audio16k[s * ALIGN_RATE // 1000 : e * ALIGN_RATE // 1000]
            words = _align_words(model, seg, line, language, s)
            ok = bool(words) and words_to_chars(line, words, japanese)
        if ok:
            aligned += 1
        else:
            estimate_line(line, japanese)
            estimated += 1
        log(f"[{k}/{len(todo)}] {'aligned  ' if ok else 'estimated'} {line.text.strip()[:40]}")
    return aligned, estimated


def _align_words(model, seg, line: Line, language: str | None, offset_ms: int) -> list[Word]:
    try:
        result = model.align(seg, line.text.strip(), language=language, verbose=None)
    except Exception:
        return []
    if result is None:
        return []
    words = []
    for w in result.all_words():
        if w.start is None or w.end is None:
            continue
        words.append(Word(w.word, offset_ms + round(w.start * 1000), offset_ms + round(w.end * 1000)))
    return words
