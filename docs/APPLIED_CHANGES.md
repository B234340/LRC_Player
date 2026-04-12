# 適用済み変更記録

日付: 2026-04-12

このファイルは、`Improvement.md` の方針に基づいてリポジトリに適用した変更の短い記録です。

- `49935a7` — chore: add architecture facade modules (domain/application/infrastructure/presentation/shared)
  - 追加ファイル:
    - `src/domain/mod.rs`
    - `src/application/mod.rs`
    - `src/infrastructure/mod.rs`
    - `src/presentation/mod.rs`
    - `src/shared/mod.rs`
  - 内容: 既存のモジュールを新しいレイヤー空間（`domain`, `application`, `infrastructure`, `presentation`, `shared`）で参照できるようにするファサードモジュールを追加しました（非破壊的）。

- `f76aa72` — feat(shared): add AppError/AppResult and thiserror dep
  - 追加ファイル: `src/shared/error.rs`
  - 変更: `Cargo.toml` に `thiserror` を追加、`src/shared/mod.rs` を更新して `shared::error` を公開。

備考:
- これらは初期の非破壊的ステップです。ファイルの大規模移動やモジュールパスの全面的なリファクタは実行していません。
- 次の推奨作業:
  1. 小規模なテストと CI を回して問題がないことを確認する
  2. `Improvement.md` の方針に従い、実際にファイルをレイヤーへ移動する（破壊的な変更。事前にブランチ作成を推奨）
  3. 非同期ランタイムの明確化（`tokio` など）やタスクマネージャ実装の追加
