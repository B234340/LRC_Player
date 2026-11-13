import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from lrc_align.cli import main  # noqa: E402
from lrc_align.core import Line, Word, estimate_line, fmt_time, parse_lrc, words_to_chars, write_lrc  # noqa: E402

SAMPLE = "[ti:Song]\n[ar:Artist]\n[offset:200]\n[00:10.00]夜に駆ける\n[00:14.00][00:30.00]沈むように\n[00:18.50]\n"


def test_parse_expands_repeats_applies_offset_and_sets_windows():
    lrc = parse_lrc(SAMPLE)
    assert lrc.tags == ["[ti:Song]", "[ar:Artist]"]
    assert [(l.start_ms, l.text) for l in lrc.lines] == [
        (9800, "夜に駆ける"),
        (13800, "沈むように"),
        (18300, ""),
        (29800, "沈むように"),
    ]
    assert lrc.lines[0].end_ms == 13800
    assert lrc.lines[-1].end_ms == 29800 + 6000
    assert not lrc.has_word_timing
    assert parse_lrc("[00:01.00]<00:01.00>あ<00:02.00>").has_word_timing


def test_fmt_time_rounds_to_centiseconds():
    assert fmt_time(0) == "00:00.00"
    assert fmt_time(61_234) == "01:01.23"
    assert fmt_time(61_236) == "01:01.24"


def test_estimate_and_write_round_trip():
    lrc = parse_lrc("[00:01.00]きょうは\n[00:10.00]x\n")
    estimate_line(lrc.lines[0], japanese=True)
    out = write_lrc(lrc)
    first = out.splitlines()[0]
    assert first.startswith("[00:01.00]<00:01.00>き<")
    assert first.count("<") == 5  # 4 chars + end
    # Speed cap: 3.25 units * 400 ms.
    assert first.endswith("<00:02.30>")
    assert out.splitlines()[1] == "[00:10.00]x"


def test_words_map_to_characters_in_order():
    line = Line(start_ms=1000, text="夜に 駆ける", end_ms=5000)
    words = [Word("夜に", 1000, 1600), Word("駆", 1700, 2200), Word("ける", 2200, 3000)]
    assert words_to_chars(line, words, japanese=True)
    assert line.source == "aligned"
    assert len(line.char_starts) == len(line.text)
    assert line.char_starts[0] == 1000
    assert line.char_starts[3] == 1700  # 駆 (after the space)
    assert line.char_starts[2] == line.char_starts[1]  # space takes the previous time
    assert line.char_end == 3000
    assert line.char_starts == sorted(line.char_starts)


def test_latin_words_match_case_insensitively():
    line = Line(start_ms=0, text="Hello, World", end_ms=3000)
    words = [Word(" hello", 0, 500), Word(" world", 800, 1400)]
    assert words_to_chars(line, words, japanese=False)
    assert line.char_starts[7] == 800  # W
    assert line.char_starts[5] == line.char_starts[4]  # "," was not aligned


def test_poor_alignment_is_rejected():
    line = Line(start_ms=0, text="あいうえおかきくけこ", end_ms=5000)
    assert not words_to_chars(line, [Word("あい", 0, 300), Word("ZZ", 300, 500)], japanese=True)
    assert line.char_starts == []


def test_cli_estimate_only_writes_backup(tmp_path):
    lrc = tmp_path / "song.lrc"
    lrc.write_text("[00:01.00]あいう\n[00:05.00]えお\n", encoding="utf-8")
    assert main([str(tmp_path / "song.flac"), str(lrc), "--estimate-only", "-q"]) == 0
    assert (tmp_path / "song.lrc.bak").read_text(encoding="utf-8").startswith("[00:01.00]あいう")
    text = lrc.read_text(encoding="utf-8")
    assert "<00:01.00>あ" in text and text.count("<") == 4 + 3
    # Already timed: refused without --force.
    assert main([str(tmp_path / "song.flac"), str(lrc), "--estimate-only", "-q"]) == 2


def test_cli_reads_shift_jis(tmp_path):
    lrc = tmp_path / "sjis.lrc"
    lrc.write_bytes("[00:01.00]夜に駆ける\n".encode("cp932"))
    out = tmp_path / "out.lrc"
    assert main(["x.wav", str(lrc), "--estimate-only", "-o", str(out), "-q"]) == 0
    assert "夜" in out.read_text(encoding="utf-8")
