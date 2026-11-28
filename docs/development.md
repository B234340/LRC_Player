# 開発者向けガイド

LRC Player をソースからビルド・テスト・パッケージするための情報です。

- [必要なもの](#必要なもの)
- [ビルドと実行](#ビルドと実行)
- [Windows 向けクロスビルド](#windows-向けクロスビルド)
- [テスト](#テスト)
- [ポータブル版を作る](#ポータブル版を作る)
- [アイコン](#アイコン)
- [構成](#構成)
- [オーディオエンジン](#オーディオエンジン)
- [UI](#ui)

---

## 必要なもの

- Rust 1.94 以上（`rustup` 推奨）
- Linux：ALSA と X11 / Wayland の開発用パッケージ（例：Debian / Ubuntu では `libasound2-dev`、
  `libxkbcommon-dev`、`libwayland-dev` など）
- Windows 向けクロスビルド：MinGW-w64（`x86_64-w64-mingw32-gcc`、`x86_64-w64-mingw32-windres`）
- パッケージ作成：Python 3

## ビルドと実行

```bash
cargo run -p lrc_player_gui              # GUI（開発ビルド）
cargo run --release -p lrc_player_gui    # GUI（リリースビルド）
cargo run -p lrc_player_cli -- --help    # CLI（再生デモ、デバイス一覧、スキャン要求）
```

開発ビルドでも依存クレートとオーディオエンジンは最適化してビルドします
（`Cargo.toml` の `[profile.dev.package.*]`）。デバッグビルドで音が途切れないようにするためです。

設定・ライブラリなどのデータは、実行ファイルと同じフォルダーの `data/` に作られます
（`target/<profile>/data/`）。

## Windows 向けクロスビルド

Linux（WSL を含む）から Windows 向けにビルドするときは GNU ツールチェーンを使います
（理由は [ADR 0002](ADR/0002-windows-gnu-ipc-fallback.md)）。

```bash
rustup target add x86_64-pc-windows-gnu
cargo build --release -p lrc_player_gui --target x86_64-pc-windows-gnu
# → target/x86_64-pc-windows-gnu/release/lrc_player_gui.exe
```

`apps/gui/build.rs` が MinGW の `windres` で、アイコンとバージョン情報を exe に埋め込みます。
`windres` が見つからない場合はアイコンなしでビルドされます（`WINDRES` 環境変数で場所を指定できます）。
生成される exe が使う DLL は Windows に標準で入っているものだけです。

## テスト

```bash
cargo test --workspace --no-fail-fast
cargo clippy --workspace --all-targets
cargo clippy -p lrc_player --target x86_64-pc-windows-gnu   # Windows 専用コードの確認
python3 -m pytest tools/lrc_align/tests                      # lrc-align（ML ライブラリ不要）
```

- オーディオエンジンのテストは、出力デバイスの代わりに「手動出力」でレンダラーを直接駆動するので、
  音声デバイスのない環境（CI、WSL）でも動きます。
- GUI のテストは [egui_kittest](https://crates.io/crates/egui_kittest) でヘッドレスに実行します。
- エンジンのタイミングに依存する一部の GUI テストは、高負荷時に時間がかかることがあります。

### スクリーンショット

画面のスクリーンショットは `#[ignore]` 付きのテストとして生成します
（wgpu のアダプターが必要。Linux では lavapipe などのソフトウェアレンダラーで動きます）。

```bash
APP_SCREENS_DIR=/tmp/screens \
  cargo test -p lrc_player --lib app::screenshot_tests -- --ignored --test-threads=1
```

画面に表示する曲名・歌詞などのデモデータはすべて架空のものです。
実在する曲の歌詞をテストやスクリーンショットに使わないでください。

## ポータブル版を作る

```bash
python3 tools/package/package_portable.py
# → dist/LRC_Player-<バージョン>-win64-portable.zip
```

- `dist` プロファイル（release ＋ シンボル削除 ＋ thin LTO）で Windows 向けにビルドし、
  1 つのフォルダー `LRC_Player-<バージョン>-win64-portable/` を zip にします。
- 中身：`LRCPlayer.exe`、`README.txt`（利用者向けの説明）、`tools/lrc_align/`。
- ビルド済みのものをまとめ直すだけなら `--no-build`。
- バージョンは `apps/gui/Cargo.toml` の `version` から取ります。

## アイコン

アイコンは egui の描画で作っています（`crates/presentation-egui/src/presentation/app_icon.rs`）。
デザインを変えたら、次のコマンドで `assets/icon/` の PNG（16〜512 px）と `lrc_player.ico` を作り直します。

```bash
cargo test -p lrc_player --lib app_icon::tests::render_app_icon -- --ignored
```

ウィンドウとタスクバーのアイコンは、埋め込んだ 256 px の PNG から設定します。

## 構成

Cargo workspace で、層ごとにクレートを分けています（[ADR 0001](ADR/0001-project-architecture.md)）。

```text
apps/
├── gui/                  # GUI の実行ファイル（lrc_player_gui）。起動処理とウィンドウ設定
└── cli/                  # CLI の実行ファイル（lrc_player_cli）
crates/
├── domain/               # ドメインモデル（I/O なし）
├── ports/                # 外部依存の抽象（trait）
├── application/          # ユースケース、DTO、設定（config.json）
├── adapters-audio/       # デコード、オーディオエンジン、DSP、出力（cpal / WASAPI 排他）
├── adapters-lyrics/      # LRC の解析、歌詞の探索、LRCLIB クライアント
├── adapters-persistence/ # 状態・プレイリスト・ライブラリ DB（SQLite）
├── adapters-scan/        # フォルダーのスキャン
└── presentation-egui/    # egui の UI（lrc_player クレート）
shared/contracts/         # 共有 DTO、IPC
tools/
├── lrc_align/            # 文字タイミング生成ツール（Python）
└── package/              # ポータブル版の作成スクリプト
assets/icon/              # アイコン
docs/                     # ドキュメントと ADR
```

## オーディオエンジン

`crates/adapters-audio/src/infrastructure/audio/engine/` にあります。

- **スレッド構成**：エンジンスレッド（デコード、コマンド処理）とオーディオコールバック（レンダラー）を
  リングバッファーでつなぎます。コールバックの中ではメモリ確保・ロック・I/O をしません。
- **信号の流れ**：正規化 → 10 バンド EQ → パラメトリック EQ（L/R 別に対応）→ クロスフィード →
  ヘッドルーム → フェード・音量 → リミッター → （必要なら）ディザー。
- **ギャップレス**：次の曲をあらかじめデコードし、同じサンプルレートなら継ぎ目なくつなぎます。
- **出力**：通常は cpal（共有モード）。Windows では WASAPI 排他モードも使え、失敗すると共有モードに戻ります。
- **信号経路**：`EngineSnapshot.signal` に、どの処理が有効か、ビットパーフェクトかを報告します。
- **自動作成**：`autoeq_fit.rs` が周波数特性ファイルの読み込みと、パラメトリック EQ のフィットを行います
  （1/48 オクターブの対数グリッド、平滑化、正規化、シェルフとピークの貪欲な初期配置、範囲付きの同時最適化）。

## UI

`crates/presentation-egui/src/presentation/` にあります。

- `ui/`：テーマ、フォント（日本語・中国語・韓国語の切り替え）、アイコン、部品、各画面のビュー。
  ビューは「props を受け取って描画し、操作を `UiAction` として返す」形です。
- `app/`：アプリの状態（`GuiApp`）、操作の処理（`dispatch.rs`）、ページ、ワーカーとの連携。
  描画以外の処理はすべて `App::logic` で行うので、ウィンドウを最小化しても再生や曲送りが続きます。
- 文字列はすべて `ui/i18n.rs`（日本語・英語）を通します。
- デスクトップ歌詞は eframe の deferred viewport です（`app/desktop_lyrics.rs`）。
