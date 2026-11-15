#!/usr/bin/env python3
"""Builds the Windows portable package: one folder, zipped.

    python3 tools/package/package_portable.py            # build (dist profile) + package
    python3 tools/package/package_portable.py --no-build # package the existing build

Output: dist/LRC_Player-<version>-win64-portable.zip containing a single top-level folder
LRC_Player-<version>-win64-portable/ with LRCPlayer.exe, the user guide and the lrc-align tool.
Settings, library data and logs go to the `data` folder next to the exe (created on first
start), so the folder can live anywhere (USB stick, Desktop) and is removed by deleting it.
"""
from __future__ import annotations

import argparse
import os
import re
import shutil
import subprocess
import sys
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
TARGET = "x86_64-pc-windows-gnu"
PROFILE = "dist"
EXE_NAME = "LRCPlayer.exe"

README = """LRC Player {version}（Windows 64bit ポータブル版）
================================================

■ 使い方
  LRCPlayer.exe をダブルクリックすると起動します。インストールは不要です。

■ データの保存場所
  設定・ライブラリ・プレイリスト・再生状態などは、LRCPlayer.exe と同じフォルダーの
  「data」フォルダーに保存されます（初回起動時に作られます）。
  フォルダーごと USB メモリーや別の PC にコピーすれば、設定ごと持ち運べます。
  ※ 書き込みできない場所（Program Files など）には置かないでください。

■ アップデート
  新しい版の LRCPlayer.exe（と tools フォルダー）で上書きしてください。
  data フォルダーはそのまま使えます。

■ アンインストール
  フォルダーごと削除してください。レジストリーなどは使っていません。

■ 文字タイミングの自動生成（任意）
  tools/lrc_align は、曲の音声から文字ごとのタイミングを作る Python ツールです。
  使い方は tools/lrc_align/README.md を参照してください（Python 3.9 以上が必要）。

■ 動作環境
  Windows 10 / 11（64bit）、OpenGL 2.0 以上に対応したグラフィックス。
"""


def version() -> str:
    text = (ROOT / "apps/gui/Cargo.toml").read_text(encoding="utf-8")
    m = re.search(r'^version\s*=\s*"([^"]+)"', text, re.M)
    if not m:
        sys.exit("version not found in apps/gui/Cargo.toml")
    return m.group(1)


def build() -> None:
    cmd = ["cargo", "build", "--profile", PROFILE, "-p", "lrc_player_gui", "--target", TARGET]
    print("+", " ".join(cmd), flush=True)
    subprocess.run(cmd, cwd=ROOT, check=True)


def copy_tool(dst: Path) -> None:
    src = ROOT / "tools/lrc_align"
    ignore = shutil.ignore_patterns("__pycache__", "*.pyc", "*.egg-info", "tests", ".pytest_cache")
    shutil.copytree(src, dst, ignore=ignore)


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--no-build", action="store_true", help="package the existing dist build")
    args = ap.parse_args()

    ver = version()
    if not args.no_build:
        build()
    target_dir = Path(os.environ.get("CARGO_TARGET_DIR") or ROOT / "target")
    exe = target_dir / TARGET / PROFILE / "lrc_player_gui.exe"
    if not exe.is_file():
        sys.exit(f"missing {exe} (run without --no-build)")

    name = f"LRC_Player-{ver}-win64-portable"
    dist = ROOT / "dist"
    stage = dist / name
    if stage.exists():
        shutil.rmtree(stage)
    stage.mkdir(parents=True)
    shutil.copy2(exe, stage / EXE_NAME)
    (stage / "README.txt").write_text(README.format(version=ver).replace("\n", "\r\n"), encoding="utf-8-sig")
    copy_tool(stage / "tools" / "lrc_align")

    archive = dist / f"{name}.zip"
    if archive.exists():
        archive.unlink()
    with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as z:
        for path in sorted(stage.rglob("*")):
            if path.is_file():
                z.write(path, path.relative_to(dist).as_posix())
    size = archive.stat().st_size / 1e6
    print(f"wrote {archive.relative_to(ROOT)} ({size:.1f} MB)")


if __name__ == "__main__":
    main()
