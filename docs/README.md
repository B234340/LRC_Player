# ドキュメント

| ドキュメント | 内容 |
|---|---|
| [ユーザーガイド](guide.md) | 使い方：ライブラリ、歌詞、デスクトップ歌詞、音質設定、イコライザー、ショートカット、データの保存場所 |
| [開発者向けガイド](development.md) | ビルド、Windows 向けクロスビルド、テスト、ポータブル版の作成、構成 |
| [変更履歴](../CHANGELOG.md) | バージョンごとの変更点 |
| [lrc-align](../tools/lrc_align/README.md) | 曲の音声から文字タイミングを作る補助ツール |
| [ADR](ADR/) | 設計判断の記録（Architecture Decision Records） |

## ADR の書き方

ADR は 1 つの判断につき 1 ファイルで、番号順に並べます。各ファイルには次の項目を書きます。

- Status（状態）
- Context（背景と課題）
- Decision（決定内容）
- Consequences（結果と影響）

既存の ADR：

- [0001 プロジェクトの構成](ADR/0001-project-architecture.md)
- [0002 Windows（x86_64-pc-windows-gnu）ビルドと IPC のフォールバック](ADR/0002-windows-gnu-ipc-fallback.md)
