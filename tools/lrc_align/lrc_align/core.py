"""Pure parts of lrc-align: LRC parsing/writing, word -> character timing, fallback estimate.

No third-party imports here, so this module is testable without the ML dependencies.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, field

_TIME = re.compile(r"\[(\d+):(\d{1,2})(?:[.:](\d{1,3}))?\]")
_TAG = re.compile(r"^\[([A-Za-z#]+):(.*)\]\s*$")
_ANGLE = re.compile(r"<\d+:\d{1,2}(?:[.:]\d{1,3})?>")

# Slowest estimated wipe (ms per weight unit), same as the player's estimate.
MAX_MS_PER_UNIT = 400
SPAN_USE = 0.95


@dataclass
class Line:
    start_ms: int
    text: str
    # End of the line's window (next line start, or start + 6 s for the last line).
    end_ms: int = 0
    # Per-character start times (len == len(text)) and the end of the last character.
    char_starts: list[int] = field(default_factory=list)
    char_end: int = 0
    # How the timing was made: "aligned", "estimated" or "" (none).
    source: str = ""


@dataclass
class Lrc:
    tags: list[str]
    lines: list[Line]
    offset_ms: int = 0

    @property
    def has_word_timing(self) -> bool:
        return self._enhanced

    _enhanced: bool = False


def _ms(m: re.Match) -> int:
    minutes, seconds, frac = m.group(1), m.group(2), m.group(3) or "0"
    frac_ms = int(frac.ljust(3, "0")[:3])
    return (int(minutes) * 60 + int(seconds)) * 1000 + frac_ms


def parse_lrc(text: str) -> Lrc:
    """Parse a (line-based) LRC. Lines with several timestamps are expanded; the `[offset:]`
    tag is applied to the times and dropped (it is baked into the output)."""
    tags: list[str] = []
    entries: list[tuple[int, str]] = []
    offset = 0
    enhanced = False
    for raw in text.splitlines():
        raw = raw.strip("﻿").rstrip("\r")
        if not raw.strip():
            continue
        times = []
        rest = raw
        while True:
            m = _TIME.match(rest)
            if not m:
                break
            times.append(_ms(m))
            rest = rest[m.end():]
        if times:
            if _ANGLE.search(rest):
                enhanced = True
            body = _ANGLE.sub("", rest)
            for t in times:
                entries.append((t, body))
            continue
        tag = _TAG.match(raw)
        if tag:
            key, value = tag.group(1).lower(), tag.group(2).strip()
            if key == "offset":
                try:
                    offset = int(value)
                except ValueError:
                    pass
                continue
            tags.append(raw.strip())
    entries.sort(key=lambda e: e[0])
    # [offset:] positive = lyrics earlier.
    lines = [Line(start_ms=max(0, t - offset), text=body) for t, body in entries]
    for i, line in enumerate(lines):
        nxt = next((l.start_ms for l in lines[i + 1:] if l.start_ms > line.start_ms), None)
        line.end_ms = nxt if nxt is not None else line.start_ms + 6000
    lrc = Lrc(tags=tags, lines=lines, offset_ms=offset)
    lrc._enhanced = enhanced
    return lrc


def fmt_time(ms: int) -> str:
    cs = (max(0, ms) + 5) // 10
    return f"{cs // 6000:02d}:{(cs // 100) % 60:02d}.{cs % 100:02d}"


def write_lrc(lrc: Lrc) -> str:
    """Enhanced LRC: `[start]<t>c<t>c...<end>` for timed lines, plain otherwise."""
    out = [*lrc.tags]
    for line in lrc.lines:
        head = f"[{fmt_time(line.start_ms)}]"
        if line.char_starts and len(line.char_starts) == len(line.text):
            parts = [head]
            for ch, t in zip(line.text, line.char_starts):
                parts.append(f"<{fmt_time(t)}>{ch}")
            parts.append(f"<{fmt_time(line.char_end)}>")
            out.append("".join(parts))
        else:
            out.append(head + line.text)
    return "\n".join(out) + "\n"


def _is_kana(c: str) -> bool:
    o = ord(c)
    return 0x3041 <= o <= 0x30FF or 0xFF66 <= o <= 0xFF9F


def _is_han(c: str) -> bool:
    o = ord(c)
    return 0x4E00 <= o <= 0x9FFF or 0x3400 <= o <= 0x4DBF or 0xF900 <= o <= 0xFAFF or c in "々〆"


def char_weight(c: str, japanese: bool) -> float:
    """Singing-time weight (same table as the player's estimate)."""
    if c in "ぁぃぅぇぉゃゅょゎァィゥェォャュョヮｧｨｩｪｫｬｭｮ":
        return 0.25
    if c in "ﾞﾟ":
        return 0.0
    if c in "ーｰ〜～":
        return 1.0
    if _is_kana(c):
        return 1.0
    if _is_han(c):
        return 2.0 if japanese else 1.0
    if "가" <= c <= "힯":
        return 1.0
    if c.isspace():
        return 0.15
    if c.isalpha():
        return 0.35
    if c.isdigit():
        return 0.8
    return 0.05


def is_japanese(lrc: Lrc) -> bool:
    return any(_is_kana(c) for line in lrc.lines for c in line.text)


def spread(chars: str, start: int, end: int, japanese: bool) -> list[int]:
    """Start times for `chars` spread over [start, end] by singing weight."""
    weights = [char_weight(c, japanese) for c in chars]
    total = sum(weights)
    if not chars:
        return []
    if total <= 0:
        return [start] * len(chars)
    out, acc = [], 0.0
    for w in weights:
        out.append(start + round((end - start) * acc / total))
        acc += w
    return out


def estimate_line(line: Line, japanese: bool) -> None:
    """Fallback: the player's estimate (weighted spread, capped speed)."""
    if not line.text.strip():
        return
    total = sum(char_weight(c, japanese) for c in line.text)
    span = line.end_ms - line.start_ms
    if total <= 0 or span <= 0:
        return
    wipe = min(span * SPAN_USE, total * MAX_MS_PER_UNIT)
    line.char_starts = spread(line.text, line.start_ms, line.start_ms + round(wipe), japanese)
    line.char_end = line.start_ms + round(wipe)
    line.source = "estimated"


@dataclass
class Word:
    text: str
    start_ms: int
    end_ms: int


def _norm(c: str) -> str:
    return c.lower()


def words_to_chars(line: Line, words: list[Word], japanese: bool) -> bool:
    """Assign per-character times from aligned words. Word text is matched to the line text in
    order (whitespace ignored); each word's span is spread over its characters by weight.
    Characters no word covered get the time of their neighbour. Returns False when too little
    of the line was matched (caller falls back to the estimate)."""
    text = line.text
    n = len(text)
    starts: list[int | None] = [None] * n
    pos = 0
    matched = 0
    end = line.start_ms
    for w in words:
        wchars = [c for c in w.text if not c.isspace()]
        if not wchars:
            continue
        idx: list[int] = []
        p = pos
        for wc in wchars:
            while p < n and (text[p].isspace() or _norm(text[p]) != _norm(wc)):
                # Skip characters the aligner dropped (punctuation), but not too far.
                if not text[p].isspace() and len(idx) == 0 and p - pos > 3:
                    break
                p += 1
            if p >= n or _norm(text[p]) != _norm(wc):
                idx = []
                break
            idx.append(p)
            p += 1
        if not idx:
            continue
        ws, we = max(w.start_ms, line.start_ms), max(w.end_ms, w.start_ms)
        ws = max(ws, end) if matched else ws
        we = max(we, ws)
        times = spread("".join(text[i] for i in idx), ws, we, japanese)
        for i, t in zip(idx, times):
            starts[i] = t
        matched += len(idx)
        end = we
        pos = p
    visible = sum(1 for c in text if not c.isspace())
    if visible == 0 or matched < 0.6 * visible:
        return False
    # Fill gaps (spaces, dropped punctuation) from the previous character; keep monotonic.
    filled: list[int] = []
    last = line.start_ms
    first_known = next((t for t in starts if t is not None), line.start_ms)
    for t in starts:
        if t is None:
            t = last if filled else first_known
        t = max(t, last)
        filled.append(t)
        last = t
    line.char_starts = filled
    line.char_end = max(end, last)
    line.source = "aligned"
    return True
