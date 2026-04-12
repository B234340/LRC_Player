//! Audio::device_watcher — 出力デバイスの変化を監視するユーティリティ
//!
//! `cpal` を用いて出力デバイス一覧やデフォルト出力の変化を定期的にポーリングし、
//! 変化があれば `DeviceWatcherMessage::DevicesChanged` を送出します。UI はこれを受けて
//! デバイス選択肢を更新できます。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait};

/// デバイス監視のイベント
#[derive(Debug, Clone)]
pub enum DeviceWatcherMessage {
    /// デバイス一覧やデフォルトデバイスが変化した
    DevicesChanged {
        devices: Vec<String>,
        default: Option<String>,
    },
}

/// デバイス監視ワーカーのハンドル
pub struct DeviceWatcherHandle {
    cancel_requested: Arc<AtomicBool>,
    receiver: Receiver<DeviceWatcherMessage>,
}

impl DeviceWatcherHandle {
    pub fn spawn(poll_interval_ms: u64) -> Self {
        let (sender, receiver) = mpsc::channel();
        let cancel_requested = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel_requested);

        thread::spawn(move || {
            let host = cpal::default_host();
            let mut last_devices: Vec<String> = Vec::new();
            let mut last_default: Option<String> = None;
            while !worker_cancel.load(Ordering::Relaxed) {
                let current_devices = host
                    .output_devices()
                    .ok()
                    .map(|iter| iter.filter_map(|d| d.name().ok()).collect::<Vec<_>>())
                    .unwrap_or_default();

                let current_default = host.default_output_device().and_then(|d| d.name().ok());

                if current_devices != last_devices || current_default != last_default {
                    let _ = sender.send(DeviceWatcherMessage::DevicesChanged {
                        devices: current_devices.clone(),
                        default: current_default.clone(),
                    });
                    last_devices = current_devices;
                    last_default = current_default;
                }

                thread::sleep(Duration::from_millis(poll_interval_ms));
            }
        });

        Self {
            cancel_requested,
            receiver,
        }
    }

    pub fn cancel(&self) {
        self.cancel_requested.store(true, Ordering::Relaxed);
    }

    pub fn drain_messages(&self) -> Vec<DeviceWatcherMessage> {
        let mut messages = Vec::new();
        while let Ok(message) = self.receiver.try_recv() {
            messages.push(message);
        }
        messages
    }
}

#[cfg(test)]
impl DeviceWatcherHandle {
    pub fn from_receiver_for_test(receiver: Receiver<DeviceWatcherMessage>) -> DeviceWatcherHandle {
        DeviceWatcherHandle {
            cancel_requested: Arc::new(AtomicBool::new(false)),
            receiver,
        }
    }
}
