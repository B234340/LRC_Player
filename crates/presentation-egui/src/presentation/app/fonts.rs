//! フォント設定を担当
//! - CJKフォールバックフォントの検出と設定

use eframe::CreationContext;
use eframe::egui;
use fontdb::{Database, Family, Source, Style};
use std::collections::HashSet;
use std::fs;
use std::sync::Arc;

use crate::presentation::app::GuiApp;

impl GuiApp {
    /// アプリケーションのフォントをセットアップします。
    /// 
    /// この関数は以下の処理を行います：
    /// 1. デフォルトのフォント定義を取得
    /// 2. システムからCJK（中国語・日本語・韓国語）フォールバックフォントを検出
    /// 3. 検出されたフォントをフォントデータとして登録
    /// 4. プロポーショナルフォントとモノスペースフォントの両方に
    ///    フォールバックフォントを追加
    /// 5. 設定したフォントをeguiコンテキストに適用
    /// 
    /// フォールバックフォントはリストの先頭に追加されるため、
    /// メインフォントで表示できない文字がある場合に使用されます。
    /// 
    /// # 引数
    /// * `cc` - eframeの作成コンテキスト（フォント設定用）
    pub(crate) fn setup_fonts(cc: &CreationContext<'_>) {
        // デフォルトのフォント定義を取得
        let mut fonts = egui::FontDefinitions::default();
        
        // システムからCJKフォールバックフォントを検出
        let fallback_fonts = Self::discover_cjk_fallback_fonts();

        // フォールバックフォントが見つかった場合のみ設定を更新
        if !fallback_fonts.is_empty() {
            // 各フォールバックフォントをフォントデータとして登録
            // キーは "Noto_Sans_CJK_SC" のような形式
            for (name, bytes) in &fallback_fonts {
                fonts
                    .font_data
                    .insert(name.clone(), Arc::new(egui::FontData::from_owned(bytes.clone())));
            }

            // プロポーショナルフォントファミリーにフォールバックを追加
            // リストの先頭に追加することで、優先的に使用されるようにする
            let family = fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default();
            
            // 逆順で追加することで、優先度の高いフォントが先頭に来るようにする
            // （candidates配列の後ろの方が優先度が低い想定）
            for (name, _) in fallback_fonts.iter().rev() {
                if !family.contains(name) {
                    family.insert(0, name.clone());
                }
            }

            // モノスペースフォントファミリーにも同様にフォールバックを追加
            let monospace = fonts.families.entry(egui::FontFamily::Monospace).or_default();
            for (name, _) in fallback_fonts.iter().rev() {
                if !monospace.contains(name) {
                    monospace.insert(0, name.clone());
                }
            }
        }

        // 設定したフォントをeguiコンテキストに適用
        cc.egui_ctx.set_fonts(fonts);
    }

    /// システムにインストールされているCJKフォールバックフォントを検出します。
    /// 
    /// この関数は以下の優先順位でフォントを検索します：
    /// 
    /// **中国語（簡体字）**:
    /// - Noto Sans CJK SC
    /// - Source Han Sans SC
    /// - Microsoft YaHei
    /// - PingFang SC
    /// 
    /// **中国語（繁体字）**:
    /// - Noto Sans CJK TC
    /// - Source Han Sans TC
    /// - Microsoft JhengHei
    /// 
    /// **日本語**:
    /// - Noto Sans CJK JP
    /// - Source Han Sans JP
    /// - Noto Sans JP
    /// - IPAPGothic
    /// - Hiragino Sans
    /// 
    /// **韓国語**:
    /// - Noto Sans CJK KR
    /// - Source Han Sans KR
    /// 
    /// **その他**:
    /// - Noto Sans SC / TC（汎用）
    /// - WenQuanYi Zen Hei（Linux用）
    /// 
    /// 各フォントはシステムのフォントデータベースから検索され、
    /// 見つかった場合はフォントデータ（バイト列）として読み込まれます。
    /// 同じフォントファミリーが重複して追加されることを防ぐため、
    /// 名前ベースの重複チェックを行います。
    /// 
    /// # 戻り値
    /// 検出されたフォントのリスト。
    /// 各要素は `(フォント識別子, フォントデータのバイト列)` のタプル。
    /// フォント識別子はファミリー名の空白をアンダースコアに置換したもの。
    pub(crate) fn discover_cjk_fallback_fonts() -> Vec<(String, Vec<u8>)> {
        // フォントデータベースを初期化
        let mut database = Database::new();
        
        // システムにインストールされている全フォントを読み込み
        database.load_system_fonts();

        // 検索対象のフォントファミリー候補リスト
        // 優先度順に並んでいる（先頭ほど優先）
        let candidates = [
            // 簡体字中国語
            "Noto Sans CJK SC",
            "Source Han Sans SC",
            "Microsoft YaHei",
            "PingFang SC",
            // 繁体字中国語
            "Noto Sans CJK TC",
            "Source Han Sans TC",
            "Microsoft JhengHei",
            // 日本語
            "Noto Sans CJK JP",
            "Source Han Sans JP",
            // 韓国語
            "Noto Sans CJK KR",
            "Source Han Sans KR",
            // 汎用フォールバック
            "Noto Sans JP",
            "Noto Sans SC",
            "Noto Sans TC",
            // Linux用
            "IPAPGothic",
            "WenQuanYi Zen Hei",
            // macOS用
            "Hiragino Sans",
        ];

        let mut loaded = Vec::new();
        
        // 重複追加を防ぐためのセット
        // キーは "Noto_Sans_CJK_SC" のような形式
        let mut seen = HashSet::new();

        // 各候補フォントを順に検索
        for family_name in &candidates {
            // フォント検索クエリを構築
            let query = fontdb::Query {
                families: &[Family::Name(family_name)],
                weight: fontdb::Weight::NORMAL,
                stretch: fontdb::Stretch::Normal,
                style: Style::Normal,
            };

            // クエリに一致するフォントを検索
            if let Some(face_id) = database.query(&query) {
                if let Some(face) = database.face(face_id) {
                    // フォントデータを取得
                    let font_bytes = match &face.source {
                        // ファイルパスから読み込み
                        Source::File(path) => fs::read(path).ok(),
                        // 埋め込みバイナリからコピー
                        Source::Binary(data) => Some(data.as_ref().as_ref().to_vec()),
                        // その他のソースは非対応
                        _ => None,
                    };

                    // フォントデータが取得できた場合
                    if let Some(bytes) = font_bytes {
                        // 識別子を生成（空白をアンダースコアに置換）
                        let key = family_name.replace(' ', "_");
                        
                        // 重複チェック
                        if seen.insert(key.clone()) {
                            loaded.push((key, bytes));
                        }
                    }
                }
            }
        }

        loaded
    }
}