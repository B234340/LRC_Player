# ファイル一覧と説明 (LRC_Player)

このドキュメントはリポジトリ内の主要ファイル／モジュールごとに「目的」と「主要な機能」を短くまとめたものです。詳細な実装は各ファイル内のコード／テストを参照してください。

---

## ルート

- [Cargo.toml](Cargo.toml): プロジェクトの Cargo 設定。依存関係とパッケージ情報を記述。
- [README.md](README.md): プロジェクト概要・ビルド／実行手順のドキュメント。
- [build_release_binaries.sh](build_release_binaries.sh): リリース用バイナリをビルドするヘルパースクリプト。
- [setup_cross_build_prereqs.sh](setup_cross_build_prereqs.sh): クロスビルド前提のセットアップスクリプト。
- [.cargo/config.toml](.cargo/config.toml): Cargo 設定（ターゲット毎の設定など）。
- [.github/workflows/release.yml](.github/workflows/release.yml): GitHub Actions のリリースワークフロー。
- [.github/workflows/release-check.yml](.github/workflows/release-check.yml): リリース前チェック用ワークフロー。
- `data/`: 実行時のキャッシュ・設定・ログ類を置くディレクトリ（アプリの永続ストアとして利用）。
- `releases/`: ビルド済みリリース成果物を格納（プラットフォーム別）。

---

## src/ — トップレベル

- [src/main.rs](src/main.rs)
  - アプリケーションのエントリ。プラットフォーム固有の環境整備（Linux の場合の GL 設定）を行い、`eframe::run_native` で UI を起動。

- [src/app.rs](src/app.rs)
  - `Application` ラッパー。`AppFacade` と `JsonStore` をまとめて初期化／永続化し、UI からのコマンド仲介やバックグラウンドのポーリングを扱う。

- [src/logging.rs](src/logging.rs)
  - 非同期ファイルロガー (`AsyncFileLogger`) を実装。ログの永続化・回転、ランタイムでのフィルタ適用、初期ログレベルの復元を行う。

- [src/ui/mod.rs](src/ui/mod.rs)
  - UI の公開エントリ（`UiShell`）とタブ群モジュールの再エクスポート。

- [src/ui/egui_app.rs](src/ui/egui_app.rs)
  - `LrcPlayerEguiApp`（`eframe::App` 実装）。トップタブ描画、各タブのレンダリング、ユーザー操作→`AppCommand` 変換、CJK フォントのインストール、フォルダピッカー利用など UI 周辺ロジックを担当。

---

## src/app/**

- [src/app/commands.rs](src/app/commands.rs)
  - UI → アプリの操作命令 `AppCommand` を定義（トラッククリック、再生操作、タブ選択、スキャン、設定変更など）。

- [src/app/events.rs](src/app/events.rs)
  - アプリ内部イベント `AppEvent` / `PlaybackEvent`。Facade → UI (または上位) に通知するためのイベント列挙。

- [src/app/view_models.rs](src/app/view_models.rs)
  - UI 表示用の ViewModel 群（`AppViewModel`, `TrackRowVm`, 各タブの `*TabVm`）。UI 側が消費する軽量 DTO を定義。

- [src/app/facade.rs](src/app/facade.rs)
  - アプリケーションの中核。`AppFacade` が次を管理する:
    - `StringInterner`, `Library`, `Queue`, `PlaylistStore`, `PlaybackModel`, `ScanModel`, 設定
    - オーディオ出力 (`BufferedAudioOutput` / `AudioDeviceOutput`)、デコーダ、デコードワーカー、スキャンワーカー
    - 永続化（`JsonStore` を通じた config/state/playlists/index の保存・復元）
    - `handle_command()` / `view_model()` / `poll_background_tasks()` / `prepare_audio()` 等の主要 API
  - 「ログスナップショットで即時表示 → バックグラウンドで hydrate（lazy-load）して詳細を埋める」設計や、スキャン／ブラウズ・バックグラウンド処理の orchestration を実装。プロジェクトで最も重要かつ大きなファイル。

---

## src/audio/**

- [src/audio/mod.rs](src/audio/mod.rs)
  - `device_watcher` と `output` モジュールの再エクスポート。

- [src/audio/output.rs](src/audio/output.rs)
  - 音声出力周りの core 実装。
  - `AudioOutputConfig`, `AudioFrameBlock`（フレームブロック表現）、`BufferedAudioOutput`（bounded queue を持つバッファ）を提供。
  - `AudioDeviceOutput` は `cpal` を用いて OS の出力ストリームを構築し、サンプル変換／チャンネルリマップ／出力ループを実装。

- [src/audio/device_watcher.rs](src/audio/device_watcher.rs)
  - 出力デバイスのホットプラグ検出用ワーカー (`DeviceWatcherHandle`)。定期ポーリングでデバイス一覧／デフォルトの変化を検知してメッセージ送信。

---

## src/decode/**

- [src/decode/mod.rs](src/decode/mod.rs)
  - `decoder` と `source` の再エクスポート。

- [src/decode/decoder.rs](src/decode/decoder.rs)
  - `Decoder`（`symphonia` を利用）: ファイル→`AudioFrameBlock` へのデコード、リサンプリング、ブロック化、`DecodeWorkerHandle` によるバックグラウンドデコードを提供。
  - デコード失敗・終了の通知をワーカー経由で返す。

- [src/decode/source.rs](src/decode/source.rs)
  - `DecodedSource`（`TrackId` とファイル `PathBuf` を保持）と `from_library` ヘルパー。

---

## src/core/**

- [src/core/mod.rs](src/core/mod.rs)
  - core サブモジュールの再エクスポート。

- [src/core/ids.rs](src/core/ids.rs)
  - 型安全な ID ラッパー群（`TrackId`, `AlbumId`, `ArtistId`, `PlaylistId`, `StrId`, `PathId`, `QueueEntryId`）と、トラックの安定キー `TrackKey` を定義。

- [src/core/time.rs](src/core/time.rs)
  - シークや位置保存の定数（`DEFAULT_SEEK_STEP_MS`, `POSITION_SAVE_THRESHOLD_MS`）と `SeekAmount`。

- [src/core/scan.rs](src/core/scan.rs)
  - ディスク走査・タグ抽出の実装。`ScanModel` と `ScanWorkerHandle` を通じ、並列（`rayon`）でファイルを解析し `TrackInput` を収集。
  - 進捗送信、キャンセル、ターゲットスキャン、scan の結果要約 (`ScanOutcome`) を提供。

- [src/core/artwork.rs](src/core/artwork.rs)
  - アートワークの LRU 風キャッシュ `ArtworkCache` と JSON 永続化（save/load）。

- [src/core/strings.rs](src/core/strings.rs)
  - `StringInterner`：文字列のインターンによるメモリ・効率化と `StrId` 管理。

- [src/core/playback.rs](src/core/playback.rs)
  - `PlaybackModel` と `PlaybackState`。再生開始／停止／シーク／一時停止等の状態管理ロジック。

- [src/core/playlists.rs](src/core/playlists.rs)
  - `Playlist` と `PlaylistStore`。システムお気に入り管理、ユーザー作成プレイリスト、トラック追加／トグル等の操作。

- [src/core/persistence.rs](src/core/persistence.rs)
  - 永続化層：`AppConfigFile`, `AppStateFile`, `PlaylistsFile`, `JsonStore`。
  - `JsonStore` は data ルートの決定、JSON の安全な書き出し（tmp→rename）、インデックスやアートワークキャッシュの読み書きを行う。

- [src/core/queue.rs](src/core/queue.rs)
  - 再生キュー `Queue` と `RepeatMode`、シャッフルロジック、エントリ操作（push/insert/remove/advance/previous）を提供。

- [src/core/library.rs](src/core/library.rs)
  - ライブラリの内部構造（`TrackTable`, `AlbumTable`, `ArtistTable`, `DirectoryTree`）と、
  - トラックの upsert（`upsert_track` / `upsert_tracks_batch`）、インデックス再構築（albums/artists）、ディレクトリツリー構築、トラックキーでの解決などを担当。
  - 大量トラック向けにバッチ処理をサポートし、パフォーマンス計測やテストも含む。

---

## src/ui/tabs/**

各タブは VM 型を定義し、`egui_app.rs` がそれを用いて描画します。主要ファイル：

- [src/ui/tabs/mod.rs](src/ui/tabs/mod.rs): タブ VM の再エクスポート。
- [src/ui/tabs/album.rs](src/ui/tabs/album.rs): アルバム一覧と展開されたアルバムの表示用型。
- [src/ui/tabs/artist.rs](src/ui/tabs/artist.rs): アーティスト一覧用型（トラックアーティスト／アルバムアーティスト等）。
- [src/ui/tabs/library.rs](src/ui/tabs/library.rs): ファイルシステム的なディレクトリ一覧用型。
- [src/ui/tabs/playlist.rs](src/ui/tabs/playlist.rs): プレイリスト一覧と選択用型。
- [src/ui/tabs/queue.rs](src/ui/tabs/queue.rs): キュー表示用型。
- [src/ui/tabs/play.rs](src/ui/tabs/play.rs): 再生（Now Playing）タブ用型。
- [src/ui/tabs/scan.rs](src/ui/tabs/scan.rs): スキャン設定・進捗表示用型。
- [src/ui/tabs/settings.rs](src/ui/tabs/settings.rs): 設定一覧表示用型。

---

## テストとユーティリティ

- 多くのモジュールにユニットテストが含まれており（例: `core/scan.rs`, `core/library.rs`, `app/facade.rs` 等）、主要な振る舞い（スキャン、アップサート、再生制御、永続化）の検証コードがある。

---

## まとめ（使い分け）

- UI 層: `src/ui/*` と `src/app/view_models.rs`（ViewModel）
- アプリ管理層: `src/app/facade.rs` と `src/app.rs`（コマンド→イベント→永続化→バックグラウンド管理）
- メディア処理: `src/decode/*`（デコード・リサンプリング）、`src/audio/*`（出力・バッファリング・デバイス制御）
- データ層: `src/core/*`（ライブラリ、インデックス、スキャン、永続化、プレイリスト、再生モデル）
- インフラ: `src/logging.rs`（非同期ログ）、スクリプトや CI（ルート直下）

詳細の深掘り（例: `AppFacade::prepare_audio()` のフローや `scan` の並列処理、オーディオ出力のリサンプリング実装など）を行う場合は、どのファイル／機能を優先して展開するか教えてください。
