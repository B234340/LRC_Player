# lrc-align

行単位の `.lrc` に、曲の音声から**文字ごとのタイミング**（カラオケ用の `<mm:ss.xx>` タグ）を付けるツールです。
LRC Player の「音声から文字タイミングを生成」から呼び出せます。コマンドラインから単体でも使えます。

## しくみ

1. **ボーカル分離**: Demucs（`htdemucs`）で伴奏を取り除きます。
2. **行ごとの強制アライメント**: LRC の行の時刻で音声を切り出し、その行の歌詞を
   stable-ts（Whisper ベース）で音声に合わせます。
3. **文字ごとの時刻**: 合わせた単語の時刻を、読みの長さ（かな＝1拍、漢字＝2拍など）で各文字に配分します。
4. 合わせられなかった行は、読みの長さから**推定**したタイミングにします（LRC Player の推定表示と同じ計算）。
5. 元の `.lrc` は `.lrc.bak` として残し、上書きします（`-o` で別ファイルにも出力できます）。

精度は曲によります。ずれた箇所は LRC Player の「文字ずらし」や手作業で直してください。

## インストール

Python 3.9 以上が必要です。

```sh
# CPU 版 PyTorch（GPU を使う場合は https://pytorch.org の手順で CUDA 版を入れる）
pip install torch torchaudio --index-url https://download.pytorch.org/whl/cpu
# このツール本体と依存ライブラリ（stable-ts, demucs, soundfile）
pip install "./tools/lrc_align[ml]"
```

- WAV / FLAC / OGG / MP3 はそのまま読めます。**m4a / AAC などは ffmpeg** が必要です（PATH に置く）。
- 初回実行時に Whisper と Demucs のモデルがダウンロードされます（`small` で約 0.5 GB）。

## 使い方

```sh
lrc-align 曲.flac 曲.lrc                  # 曲.lrc を上書き（曲.lrc.bak に元を保存）
lrc-align 曲.flac 曲.lrc -o 曲.karaoke.lrc
lrc-align 曲.mp3 曲.lrc --model medium    # 精度重視（遅い）
lrc-align 曲.mp3 曲.lrc --no-separate     # ボーカル分離なし（速いが精度は下がる）
lrc-align 曲.mp3 曲.lrc --estimate-only   # 音声解析なし（推定のみ・数秒）
```

| オプション | 説明 |
|---|---|
| `--model` | Whisper モデル: `tiny` / `base` / `small`（既定）/ `medium` / `large-v3` |
| `--language` | 言語コード（`ja`, `en`, `zh` …）。省略時はかなを含めば `ja`、それ以外は自動判定 |
| `--device` | `cpu`（既定）/ `cuda` |
| `--force` | すでに文字タイミングがある LRC もやり直す |

終了コード: 0 = 成功、1 = 音声・モデルのエラー、2 = 入力 LRC の問題、3 = 依存ライブラリが入っていない。

処理時間の目安（`small`、CPU 8 コア）: 曲の長さとほぼ同じ（4 分の曲で 4〜5 分）。メモリは約 2 GB 使います。
初回だけモデルのダウンロード（約 0.55 GB）に時間がかかります。

動作確認: Windows の音声合成で作った日本語 4 行（伴奏の代わりに低音を重ねたもの）で、
4 行すべてがアライメントされ、各行の歌い出しの誤差は 0.07 秒以内でした。
実際の歌声の精度は曲によって変わります。

## LRC Player から使う

設定 → 歌詞 →「文字タイミング生成コマンド」に次のどれかを入力します。

- `lrc-align`（`pip install` でコマンドが PATH に入った場合）
- `python -m lrc_align`
- オプション付き: `lrc-align --model medium`

再生中の曲で、歌詞コントロールの「⋮」→「音声から文字タイミングを生成」を押します。
曲と `.lrc` のパスが末尾に付けて実行され、終わると歌詞が読み込み直されます。

## テスト

```sh
pip install pytest
python -m pytest tools/lrc_align/tests
```

`tests/` は ML ライブラリなしで動く部分（LRC の読み書き、単語→文字の割り当て、推定、CLI）を確認します。
