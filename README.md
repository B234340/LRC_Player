# LRC Player

歌詞（LRC）の表示に強い、デスクトップ向けの音楽プレイヤーです。
行ごと・文字ごとのタイミングでカラオケのように歌詞を塗りつぶし、音質の調整機能も備えています。
Windows を主な対象とし、macOS と Linux でもソースからビルドして使えます。

*A desktop music player focused on synced lyrics (LRC) — karaoke-style word timing, a desktop
lyrics window, loudness normalization, a parametric EQ with automatic fitting to a target curve,
and WASAPI exclusive output. The UI is Japanese by default (English is available).*

## 主な機能

**歌詞**
- 行単位・文字単位（`<mm:ss.xx>` タグ）の LRC を表示。文字単位はカラオケのように塗りつぶします
- 行単位の LRC も、読みの長さから文字のタイミングを推定して塗りつぶし（表示のみ・ファイルは変更しません）
- 歌詞全体を行単位・文字単位でずらす機能、曲ごとのオフセット
- デスクトップ歌詞：ほかのアプリの上に浮かぶ小さな歌詞ウィンドウ（透明度・アルバムアート・クリック透過）
- LRCLIB からの歌詞取得（オンにしたときだけ）と、歌詞のない曲の一括取得
- 曲の音声から文字タイミングを作る補助ツール [`lrc-align`](tools/lrc_align/README.md)（任意・Python）

**音質**
- 音量の正規化（ReplayGain タグ、なければ EBU R128 で自動測定）
- 10 バンドのグラフィック EQ と、最大 20 フィルターのパラメトリック EQ
- パラメトリック EQ の自動作成：ターゲットと測定データ（.txt / .csv）を読み込み、近づけるフィルターを計算。L/R 別の適用にも対応
- AutoEq / Equalizer APO 形式の読み込み、クロスフィード、自動ヘッドルーム、ディザー
- WASAPI 排他モード（Windows）と、再生中の信号経路の表示（ビットパーフェクトかどうか）
- ギャップレス再生

**ライブラリ**
- フォルダーを登録してスキャン（MP3 / FLAC / WAV / AAC / M4A / OGG）
- ライブラリ・アルバム・アーティスト・プレイリスト・再生キュー、お気に入り
- 検索と並べ替え、キーボードショートカット、メディアキー、ドラッグ＆ドロップ

## ダウンロードと起動（Windows）

1. ポータブル版の zip（`LRC_Player-<バージョン>-win64-portable.zip`）を展開します。
2. フォルダー内の `LRCPlayer.exe` を起動します。インストールは不要です。
3. 左のメニュー「フォルダー」で音楽フォルダーを追加すると、ライブラリが作られます。

設定やライブラリは exe と同じフォルダーの `data` に保存されるので、フォルダーごと持ち運べます。
削除するときはフォルダーごと消してください（レジストリは使いません）。

ポータブル版は [開発者向けガイド](docs/development.md#ポータブル版を作る) の手順でソースから作れます。

## 使い方

画面の説明、歌詞ファイルの置き方、音質設定、デスクトップ歌詞、ショートカットなどは
**[ユーザーガイド](docs/guide.md)** にまとめています。

歌詞ファイルは、曲と同じフォルダーに同じ名前で置くのがいちばん簡単です。

```text
Music/
├── 01 夜の駅.flac
└── 01 夜の駅.lrc
```

## プライバシー

- インターネットに接続するのは、歌詞の取得（LRCLIB）を使うときだけです。
  自動取得は初期状態でオフで、一括取得は実行前に確認を表示します。
  送信するのは曲名・アーティスト名・アルバム名・再生時間です。
- 再生履歴などを外部に送信することはありません。

## 動作環境

- Windows 10 / 11（64bit）、OpenGL 2.0 以上に対応したグラフィックス
- macOS / Linux：ソースからビルドして利用できます（排他モードは Windows のみ）

## ソースからビルド

[Rust](https://www.rust-lang.org/)（1.94 以上）が必要です。

```bash
cargo run --release -p lrc_player_gui
```

Linux から Windows 向けにクロスビルドする方法、テスト、パッケージの作り方は
**[開発者向けガイド](docs/development.md)** を参照してください。

## ドキュメント

- [ユーザーガイド](docs/guide.md)
- [開発者向けガイド](docs/development.md)
- [変更履歴](CHANGELOG.md)
- [lrc-align（文字タイミング生成ツール）](tools/lrc_align/README.md)
- [設計判断の記録（ADR）](docs/ADR/)

## ライセンス

未定です（決まり次第、このリポジトリに LICENSE を追加します）。
