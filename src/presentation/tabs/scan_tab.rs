use crate::presentation::egui_app::GuiApp;
use eframe::egui;

/// スキャン設定画面のUIを描画する関数
/// 
/// ライブラリスキャンの対象ディレクトリ、LRC歌詞ファイルのスキャン対象、
/// 除外パスの設定を表示し、スキャンの進行状況を確認できる
pub fn ui(ui: &mut egui::Ui, app: &mut GuiApp) {
    let vm = &app.app_vm;  // アプリケーションのビューモデルを参照
    
    // --- ヘッダーと説明 ---
    ui.heading("Scan");
    ui.label("Configure scan targets and exclude patterns, then start or cancel the scan.");
    ui.label("Scan can discover audio files and related .lrc files; LRC directories are also supported.");
    ui.separator();

    // --- スキャン対象ディレクトリ一覧（音声ファイル用）---
    ui.group(|ui| {
        ui.label("Scan targets:");
        if vm.scan.scan_targets.is_empty() {
            ui.label("No scan targets configured.");
        } else {
            // 設定されているスキャン対象を箇条書きで表示
            for target in vm.scan.scan_targets.iter() {
                ui.label(format!("• {}", target));
            }
        }
    });

    // --- LRCスキャン対象ディレクトリ一覧（歌詞ファイル用）---
    ui.group(|ui| {
        ui.label("LRC scan targets:");
        if vm.scan.scan_lrc_targets.is_empty() {
            ui.label("No LRC scan targets configured.");
        } else {
            // LRCファイル専用のスキャン対象を表示
            for target in vm.scan.scan_lrc_targets.iter() {
                ui.label(format!("• {}", target));
            }
        }
    });

    // --- 除外パス一覧（スキャン対象外のディレクトリ/ファイル）---
    ui.group(|ui| {
        ui.label("Exclude paths:");
        if vm.scan.scan_excludes.is_empty() {
            ui.label("No exclude paths configured.");
        } else {
            // スキャンから除外するパスを表示
            for excludes in vm.scan.scan_excludes.iter() {
                ui.label(format!("• {}", excludes));
            }
        }
    });

    ui.separator();

    // --- 前回のスキャン結果ステータス表示 ---
    if let Some(status) = vm.scan.last_scan_status.as_deref() {
        ui.label(format!("Last scan status: {}", status));
    } else {
        ui.label("No scan activity recorded yet.");  // まだスキャンが実行されていない
    }

    // --- スキャン進行状況の表示（実行中の場合）---
    // スキャンは3段階で進行する：
    // 1. Scanned: ファイルシステムから発見されたファイル数
    // 2. Parsed: メタデータを解析できたファイル数
    // 3. Registered: ライブラリに登録されたファイル数
    if app.app_vm.scan.scan_scanned.is_some()
        || app.app_vm.scan.scan_parsed.is_some()
        || app.app_vm.scan.scan_registered.is_some()
    {
        ui.separator();
        ui.heading("Scan progress");
        
        if let Some(scanned) = app.app_vm.scan.scan_scanned {
            ui.label(format!("Scanned: {}", scanned));  // 発見ファイル数
        }
        if let Some(parsed) = app.app_vm.scan.scan_parsed {
            ui.label(format!("Parsed: {}", parsed));  // 解析完了ファイル数
        }
        if let Some(registered) = app.app_vm.scan.scan_registered {
            ui.label(format!("Registered: {}", registered));  // 登録完了ファイル数
        }
    }

    ui.separator();
    
    ui.heading("Scan Controls");
    ui.label("Configure scan targets and exclude paths, then press Request scan.");

    ui.group(|ui| {
        ui.label("Audio scan target path:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut app.scan_target_input);
            if ui.button("Add target").clicked() {
                app.add_scan_target();
            }
        });

        if !app.scan_targets.is_empty() {
            let targets = app.scan_targets.clone();
            for (index, target) in targets.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("• {}", target));
                    if ui.button("Remove").clicked() {
                        app.remove_scan_target(index);
                    }
                });
            }
        }
    });

    ui.group(|ui| {
        ui.label("LRC scan target path:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut app.scan_lrc_target_input);
            if ui.button("Add LRC target").clicked() {
                app.add_scan_lrc_target();
            }
        });

        if !app.scan_lrc_targets.is_empty() {
            let targets = app.scan_lrc_targets.clone();
            for (index, target) in targets.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("• {}", target));
                    if ui.button("Remove").clicked() {
                        app.remove_scan_lrc_target(index);
                    }
                });
            }
        }
    });

    ui.group(|ui| {
        ui.label("Exclude path:");
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut app.scan_exclude_input);
            if ui.button("Add exclude").clicked() {
                app.add_scan_exclude();
            }
        });

        if !app.scan_excludes.is_empty() {
            let excludes = app.scan_excludes.clone();
            for (index, exclude) in excludes.iter().enumerate() {
                ui.horizontal(|ui| {
                    ui.label(format!("• {}", exclude));
                    if ui.button("Remove").clicked() {
                        app.remove_scan_exclude(index);
                    }
                });
            }
        }
    });

    ui.separator();
    ui.horizontal(|ui| {
        if ui.button("Start scan").clicked() {
            app.request_scan();
        }
        if ui.button("Cancel scan").clicked() {
            app.request_cancel();
        }
    });
    
    ui.separator();
    
    // --- ライブラリクリアボタン ---
    // スキャンしたライブラリデータをすべて削除する
    ui.horizontal(|ui| {
        if ui.button("Clear library").clicked() {
            app.clear_library();
        }
    });
}