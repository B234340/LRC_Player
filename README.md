# LRC_Player

Rust で実装された LRC 対応プレイヤーです。  
現状は Cargo workspace 構成で、`apps`（実行エントリ）と `crates`（レイヤー別ライブラリ）に分割されています。

## 現在の構成

```text
LRC_Player/
├── Cargo.toml                               # workspace ルート。member crate を定義
├── README.md                                # このファイル
├── .cargo/                                  # Cargo ローカル設定
├── .github/                                 # CI / 運用設定
│   ├── instructions/                        # 開発・運用向け補助ドキュメント
│   └── workflows/                           # GitHub Actions ワークフロー
├── apps/                                    # 実行バイナリ群（Composition Root）
│   ├── cli/
│   │   ├── Cargo.toml                       # CLI アプリ crate 定義
│   │   └── src/main.rs                      # CLI エントリポイント
│   └── gui/
│       ├── Cargo.toml                       # GUI アプリ crate 定義
│       └── src/main.rs                      # GUI エントリポイント
├── crates/                                  # レイヤー別ライブラリ群
│   ├── domain/
│   │   ├── Cargo.toml                       # ドメイン層 crate 定義
│   │   └── src/lib.rs                       # domain 公開モジュール入口
│   ├── ports/
│   │   ├── Cargo.toml                       # ポート層 crate 定義
│   │   └── src/lib.rs                       # ports 公開モジュール入口
│   ├── application/
│   │   ├── Cargo.toml                       # アプリケーション層 crate 定義
│   │   └── src/lib.rs                       # usecase / dto / vm の公開入口
│   ├── adapters-persistence/
│   │   ├── Cargo.toml                       # 永続化アダプター crate 定義
│   │   └── src/lib.rs                       # persistence 実装の公開入口
│   ├── adapters-audio/
│   │   ├── Cargo.toml                       # オーディオ出力アダプター crate 定義
│   │   └── src/lib.rs                       # audio 実装の公開入口
│   ├── adapters-scan/
│   │   ├── Cargo.toml                       # スキャンアダプター crate 定義
│   │   └── src/lib.rs                       # scan 実装の公開入口
│   ├── adapters-lyrics/
│   │   ├── Cargo.toml                       # 歌詞探索/解析アダプター crate 定義
│   │   └── src/lib.rs                       # lyrics 実装の公開入口
│   ├── presentation-egui/
│   │   ├── Cargo.toml                       # egui プレゼンテーション crate 定義
│   │   ├── src/lib.rs                       # presentation 公開入口
│   │   ├── src/presentation/                # 画面/タブ/アクション実装
│   │   ├── tests/                           # 統合テスト群
│   │   ├── bench/10k_tracks/generate.rs     # 10k データ生成ベンチ補助
│   │   └── benches/ten_k_tracks.rs          # Criterion ベンチ本体
│   └── sample_data/
│       └── test.lrc                         # テスト用 LRC サンプル
├── shared/
│   └── contracts/
│       ├── Cargo.toml                       # shared 契約 crate 定義
│       └── src/
│           ├── lib.rs                       # shared 公開モジュール入口
│           ├── dto.rs                       # 共有 DTO 定義
│           ├── error.rs                     # 共有エラー型
│           ├── ipc.rs                       # IPC プロトコル/通信
│           └── result.rs                    # 共有 Result エイリアス
├── docs/
│   ├── README.md                            # ドキュメント入口
│   └── ADR/                                 # アーキテクチャ決定記録
└── target/                                  # ビルド成果物（自動生成）
```

## Workspace メンバー

- `crates/domain`
- `crates/ports`
- `crates/application`
- `crates/adapters-persistence`
- `crates/adapters-audio`
- `crates/adapters-scan`
- `crates/adapters-lyrics`
- `shared/contracts`
- `crates/presentation-egui`
- `apps/gui`
- `apps/cli`

## 実行と検証

GUI 起動:

```bash
cargo run -p lrc_player_gui
```

CLI 起動:

```bash
cargo run -p lrc_player_cli -- --help
```

workspace 全体のビルド確認:

```bash
cargo check --workspace
```

既存テスト実行:

```bash
cargo test --workspace
```

## 補足

- `target/` は生成物のため通常は編集対象外です。
- `crates/presentation-egui` は現状、UI と統合テストを集約する主要 crate です。
