use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use cpal::traits::{HostTrait, DeviceTrait};

#[derive(Debug, Clone)]
pub enum DeviceWatcherMessage {
    DevicesChanged { devices: Vec<String>, default: Option<String> },
}

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

                let current_default = host
                    .default_output_device()
                    .and_then(|d| d.name().ok());

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

        Self { cancel_requested, receiver }
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
