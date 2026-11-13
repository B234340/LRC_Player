"""Command line: lrc-align AUDIO LRC [-o OUT]."""

from __future__ import annotations

import argparse
import os
import shutil
import sys

from .core import estimate_line, is_japanese, parse_lrc, write_lrc


def read_text(path: str) -> str:
    raw = open(path, "rb").read()
    for enc in ("utf-8-sig", "cp932", "gb18030"):
        try:
            return raw.decode(enc)
        except UnicodeDecodeError:
            continue
    return raw.decode("utf-8", errors="replace")


def write_with_backup(path: str, text: str) -> str | None:
    """Writes `text` to `path` (atomic replace); an existing file is kept as `path.bak`."""
    backup = None
    if os.path.exists(path):
        backup = path + ".bak"
        shutil.copy2(path, backup)
    tmp = path + ".tmp"
    with open(tmp, "w", encoding="utf-8", newline="\n") as f:
        f.write(text)
    os.replace(tmp, path)
    return backup


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        prog="lrc-align",
        description="Add per-character (karaoke) timing to a line-based LRC by aligning the "
        "lyrics to the vocals of the song.",
    )
    p.add_argument("audio", help="audio file (wav/flac/ogg/mp3; other formats need ffmpeg)")
    p.add_argument("lrc", help="line-based .lrc to time")
    p.add_argument("-o", "--output", help="write here instead of replacing LRC (LRC is kept as LRC.bak)")
    p.add_argument("--model", default="small", help="Whisper model: tiny/base/small/medium/large-v3 (default: small)")
    p.add_argument("--language", default=None, help="language code (ja, en, zh, ...); default: detect")
    p.add_argument("--device", default="cpu", help="torch device (cpu, cuda)")
    p.add_argument("--no-separate", action="store_true", help="skip vocal separation (faster, less accurate)")
    p.add_argument("--estimate-only", action="store_true", help="no audio analysis: write the weighted estimate")
    p.add_argument("--force", action="store_true", help="re-time an LRC that already has word timing")
    p.add_argument("-q", "--quiet", action="store_true")
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    log = (lambda _m: None) if args.quiet else (lambda m: print(m, file=sys.stderr, flush=True))
    try:
        lrc = parse_lrc(read_text(args.lrc))
    except OSError as e:
        print(f"error: {e}", file=sys.stderr)
        return 2
    if not any(l.text.strip() for l in lrc.lines):
        print("error: the LRC has no timed lyric lines", file=sys.stderr)
        return 2
    if lrc.has_word_timing and not args.force:
        print("error: the LRC already has word timing (use --force to re-time it)", file=sys.stderr)
        return 2
    japanese = is_japanese(lrc)
    language = args.language or ("ja" if japanese else None)

    if args.estimate_only:
        for line in lrc.lines:
            estimate_line(line, japanese)
        aligned, estimated = 0, sum(1 for l in lrc.lines if l.source == "estimated")
    else:
        from . import audio

        try:
            wav, sr = audio.load_audio(args.audio)
            if args.no_separate:
                mono, rate = wav.mean(0), sr
            else:
                mono, rate = audio.separate_vocals(wav, sr, args.device, log)
            audio16k = audio.to_align_rate(mono, rate)
            aligned, estimated = audio.align_lines(lrc, audio16k, japanese, args.model, language, args.device, log)
        except audio.MissingDependency as e:
            print(f"error: {e}", file=sys.stderr)
            return 3
        except Exception as e:  # decoding / model errors
            print(f"error: {e}", file=sys.stderr)
            return 1

    out_path = args.output or args.lrc
    backup = write_with_backup(out_path, write_lrc(lrc))
    msg = f"{aligned} lines aligned, {estimated} estimated -> {out_path}"
    if backup:
        msg += f" (previous: {backup})"
    print(msg)
    return 0
