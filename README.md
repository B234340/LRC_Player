# LRC Player

Rust で実装するポータブル音楽プレイヤーの作業リポジトリです。

要件と内部設計は次の文書に整理しています。

- [Requirements_Definition.md](../Requirements_Definition.md)
- [Specification.md](../Specification.md)

現時点では、仕様書のモジュール構成に沿ったビルド可能な基盤を実装しています。実際の GUI、音声出力、デコード、タグ解析は今後の実装対象で、現在は責務分離と状態モデルを先に固めています。

## 現在の構成

```text
src/
  main.rs
  app.rs
  app/
    commands.rs
    events.rs
    facade.rs
    view_models.rs
  core/
    artwork.rs
    ids.rs
    library.rs
    persistence.rs
    playback.rs
    playlists.rs
    queue.rs
    scan.rs
    strings.rs
    time.rs
  audio/
    mixer.rs
    output.rs
    ring_buffer.rs
  decode/
    decoder.rs
    source.rs
  ui/
    tabs/
```

主な実装済み範囲は次のとおりです。

- Queue、Playback、Scan の状態モデル
- `AppCommand` ベースの状態更新経路
- JSON 永続化ヘルパ
- ローカル release 出力スクリプト
- GitHub Actions による multi-target release ビルド

## 開発環境

- Rust stable
- Cargo
- Linux でのローカル開発を前提

このリポジトリでは Rust edition 2024 を使用しています。

## ローカル開発

通常のビルド:

```bash
cargo build
```

テスト実行:

```bash
cargo test
```

実行:

```bash
cargo run
```

## ローカル release 出力

release 実行ファイルのローカル出力には次のスクリプトを使います。

```bash
./build_release_binaries.sh
```

デフォルト target:

- `x86_64-unknown-linux-gnu`
- `x86_64-pc-windows-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

特定 target のみ指定する場合:

```bash
TARGETS="x86_64-unknown-linux-gnu x86_64-pc-windows-gnu" ./build_release_binaries.sh
```

出力先は `releases/<target>/` です。

## クロスビルド前提

ローカルの前提確認には次のスクリプトを使います。

```bash
./setup_cross_build_prereqs.sh
```

この環境での想定は次のとおりです。

- Linux: そのままビルド可能
- Windows GNU: `x86_64-w64-mingw32-gcc` が必要
- macOS: `osxcross` と Apple SDK が必要

Cargo の target 別 linker 設定は `.cargo/config.toml` に置いています。

### Ubuntu/Debian で Windows GNU を有効化する例

```bash
sudo apt-get update
sudo apt-get install -y gcc-mingw-w64-x86-64 binutils-mingw-w64-x86-64
```

### macOS クロスビルドについて

Linux 上で `x86_64-apple-darwin` と `aarch64-apple-darwin` をローカル生成するには、`osxcross` と手元で用意した Apple SDK が必要です。これらが未導入でも、GitHub Actions 側では macOS runner 上で同じ target 名の release ビルドを行います。

## GitHub Actions

release workflow は `.github/workflows/release.yml` にあります。
事前検証 workflow は `.github/workflows/release-check.yml` にあります。

動作は次のとおりです。

- `Release Validation` は `push` と `pull_request` で自動実行
- `Release Validation` は release と同じ target 構成で「実際に release build できるか」を検証
- `Release Validation` は先に `cargo test` を実行し、その後 target ごとの release build を確認
- `workflow_dispatch` で手動実行可能
- `v*` タグ push で release ビルドを実行
- 次の target をビルド
  - `x86_64-unknown-linux-gnu`
  - `x86_64-pc-windows-gnu`
  - `x86_64-apple-darwin`
  - `aarch64-apple-darwin`
- 各 target の実行ファイルを archive 化して artifact として保存
- タグ実行時は GitHub Release を自動作成または更新し、archive を添付

自動 Releases ビルドの使い方:

```bash
git tag v0.1.0
git push origin v0.1.0
```

生成される成果物:

- `lrc_player-x86_64-unknown-linux-gnu.tar.gz`
- `lrc_player-x86_64-pc-windows-gnu.zip`
- `lrc_player-x86_64-apple-darwin.tar.gz`
- `lrc_player-aarch64-apple-darwin.tar.gz`

タグなしで確認したい場合は、通常の push / pull request で `Release Validation` が自動実行されます。手動確認したい場合は GitHub Actions の `Release Validation` または `Release Builds` を `workflow_dispatch` で実行できます。`Release Builds` をタグなしで手動実行した場合は artifact 生成まで行い、GitHub Release への添付は行いません。

## 現状の注意点

- 現在のアプリ本体はアーキテクチャ骨格が中心です
- GUI、音声再生、ファイルスキャン、タグ解析はスタブまたは未実装です
- `cargo build` と `cargo test` は通ります
- 一部モジュールは今後利用予定のため、現時点では未使用警告が出ます

## 直近の作業候補

- 実際の GUI フレームワーク導入
- ライブラリスキャンとタグ解析の実装
- 音声デコードと出力パイプラインの実装
- release 成果物の圧縮と配布形式の整備