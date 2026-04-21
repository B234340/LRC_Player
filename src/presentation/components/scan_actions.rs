//! スキャン操作に関する処理を提供するモジュール。
//!
//! - スキャン要求/キャンセル
//! - スキャン対象の追加/削除
//! - LRC/除外対象の追加/削除

use crate::presentation::egui_app::GuiApp;
use crate::shared::ipc as ipc_mod;

pub fn request_scan(app: &mut GuiApp) {
    let paths = app.scan_targets.clone();
    let lrc_paths = app.scan_lrc_targets.clone();
    if paths.is_empty() && lrc_paths.is_empty() {
        app.last_scan_status = Some("No scan targets configured".to_string());
        app.save_state();
        app.update_app_vm();
        return;
    }

    app.last_scan_status = Some("Requested scan".to_string());
    app.save_state();
    app.update_app_vm();

    let socket = ipc_mod::default_socket_path();
    let request = ipc_mod::ScanRequest {
        paths,
        lrc_paths,
        excludes: app.scan_excludes.clone(),
        cancel: false,
    };
    std::thread::spawn(move || {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = rt.block_on(async move {
                ipc_mod::send_scan_request(&socket, &request).await
            });
        }
    });
}

pub fn request_cancel(app: &mut GuiApp) {
    app.last_scan_status = Some("Requested scan cancel".to_string());
    app.save_state();
    app.update_app_vm();

    let socket = ipc_mod::default_socket_path();
    let request = ipc_mod::ScanRequest {
        paths: Vec::new(),
        lrc_paths: Vec::new(),
        excludes: app.scan_excludes.clone(),
        cancel: true,
    };
    std::thread::spawn(move || {
        if let Ok(rt) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            let _ = rt.block_on(async move {
                ipc_mod::send_scan_request(&socket, &request).await
            });
        }
    });
}

pub fn add_scan_target(app: &mut GuiApp) {
    let target = app.scan_target_input.trim().to_string();
    if !target.is_empty() {
        app.scan_targets.push(target);
        app.scan_target_input.clear();
        app.save_state();
        app.update_app_vm();
    }
}

pub fn remove_scan_target(app: &mut GuiApp, index: usize) {
    if index < app.scan_targets.len() {
        app.scan_targets.remove(index);
        app.save_state();
        app.update_app_vm();
    }
}

pub fn add_scan_lrc_target(app: &mut GuiApp) {
    let target = app.scan_lrc_target_input.trim().to_string();
    if !target.is_empty() {
        app.scan_lrc_targets.push(target);
        app.scan_lrc_target_input.clear();
        app.save_state();
        app.update_app_vm();
    }
}

pub fn remove_scan_lrc_target(app: &mut GuiApp, index: usize) {
    if index < app.scan_lrc_targets.len() {
        app.scan_lrc_targets.remove(index);
        app.save_state();
        app.update_app_vm();
    }
}

pub fn add_scan_exclude(app: &mut GuiApp) {
    let exclude = app.scan_exclude_input.trim().to_string();
    if !exclude.is_empty() {
        app.scan_excludes.push(exclude);
        app.scan_exclude_input.clear();
        app.save_state();
        app.update_app_vm();
    }
}

pub fn remove_scan_exclude(app: &mut GuiApp, index: usize) {
    if index < app.scan_excludes.len() {
        app.scan_excludes.remove(index);
        app.save_state();
        app.update_app_vm();
    }
}
