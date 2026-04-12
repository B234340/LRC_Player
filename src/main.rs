//! LRC Player エントリポイント
//!
//! このファイルはアプリケーションの最小限の起動処理のみを含みます。
//! - プラットフォーム固有の環境調整（Linux のソフトウェアレンダラ回避）
//! - `Application` の初期化（アプリ本体ロジックは `src/app` 配下）
//! - `eframe::run_native` による UI スレッドの開始
//!
//! 注: 実行時の詳細な初期化や永続化は `Application` / `AppFacade` に委譲されています。

#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod app;
mod audio;
mod core;
mod logging;
mod decode;
mod ui;

use app::Application;
use ui::egui_app::LrcPlayerEguiApp;

/// アプリケーションのエントリポイント。
///
/// Linux 環境では GL ドライバ周りの問題回避のため、環境変数を一時的に設定してから UI を起動します。
fn main() {
    // Linux でハードウェアレンダラが問題を起こす環境向けの回避策。
    // ここは unsafe ブロックを使って直接環境変数を書き換えていますが、プロセス全体に影響します。
    #[cfg(target_os = "linux")]
    unsafe {
        // `LIBGL_ALWAYS_SOFTWARE=1` によりソフトウェアレンダラ（llvmpipe）を使用
        std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
        // ドライバローダのオーバーライドで llvmpipe 指定
        std::env::set_var("MESA_LOADER_DRIVER_OVERRIDE", "llvmpipe");
    }

    // `Application` は UI と内部モデル（Facade/Store 等）をまとめる薄いラッパー
    let application = Application::default();
    let options = eframe::NativeOptions::default();

    // eframe によるネイティブウィンドウ起動。クロージャで `App` を生成。
    if let Err(error) = eframe::run_native(
        "LRC Player",
        options,
        Box::new(|cc| Ok(Box::new(LrcPlayerEguiApp::new(cc, application)))),
    ) {
        // GUI 起動に失敗した場合は標準エラーへ出力するだけ。
        eprintln!("failed to start UI: {error}");
    }
}
