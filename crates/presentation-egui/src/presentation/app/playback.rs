//! 再生バックエンドの制御を担当
//! - 再生スレッドの起動/停止
//! - ステータスポーリング
//! - デバイスホットプラグ監視

use std::path::PathBuf;
use std::time::{Duration, Instant};

use lrc_player_application::infrastructure::audio;
use lrc_player_application::infrastructure::decode::SymphoniaDecoder;
use lrc_player_domain::model::PlaybackState;
use lrc_player_ports::audio_output::{AudioFormat, AudioOutput, SampleFormat};

use crate::presentation::app::GuiApp;

impl GuiApp {
    pub(crate) fn request_play(&mut self) {
        let path = PathBuf::from(self.play_path.clone());

        if path.as_os_str().is_empty() {
            self.play_status = Some("Play path is empty.".to_string());
            return;
        }
        if !path.exists() {
            self.play_status = Some(format!("Play path does not exist: {}", path.display()));
            return;
        }

        let is_seeking = self.playback_state == PlaybackState::Seeking;

        if self.playback_thread_handle.is_some() {
            self.signal_playback_stop();
            self.pending_play_request = true;
            if !is_seeking {
                self.playback_state = PlaybackState::Buffering;
            }
            self.play_status = Some("Stopping previous playback...".to_string());
            return;
        }

        self.pending_play_request = false;
        if !is_seeking {
            self.playback_state = PlaybackState::Buffering;
        }
        self.play_status = Some(format!("Starting playback: {}", path.display()));
        self.playback_start_instant = Some(Instant::now());
        self.playback_start_ms = self.play_position_ms;
        self.save_state();

        let start_ms = Some(self.play_position_ms);

        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
        self.playback_status_receiver = Some(status_rx);
        self.playback_cancel_sender = Some(stop_tx);

        let preferred_device = self
            .selected
            .and_then(|idx| self.device_names.get(idx).cloned());
        let taps = self.taps;

        let handle = std::thread::spawn(move || {
            let mut output = audio::CpalOutput::new();

            let requested = AudioFormat {
                sample_rate: 44_100,
                channels: 2,
                sample_format: SampleFormat::F32,
            };

            let actual = match output.start_with_device(requested, preferred_device) {
                Ok(fmt) => fmt,
                Err(e) => {
                    let _ = status_tx.send(format!("Playback failed to start output: {}", e));
                    return;
                }
            };

            let selected_device_name = output.selected_device_name();
            if let Some(device_name) = selected_device_name {
                let _ = status_tx.send(format!("Playback device: {}", device_name));
            } else {
                let _ = status_tx.send("Playback device: <default>".to_string());
            }

            let producer_tx = match output.producer_sender() {
                Some(tx) => tx,
                None => {
                    let _ = status_tx.send("Playback failed: producer channel unavailable".to_string());
                    let _ = output.stop();
                    return;
                }
            };

            let pool = output.pool_handle();
            let chunk_frames = 2048;

            let _ = status_tx.send(format!(
                "Playback running: {} Hz / {} ch",
                actual.sample_rate, actual.channels
            ));

            let decode_handle = audio::decoder::spawn_decode_to_producer(
                path,
                chunk_frames,
                actual.sample_rate,
                actual.channels,
                producer_tx,
                pool,
                taps,
                start_ms,
                stop_rx.clone(),
            );

            loop {
                if stop_rx.try_recv().is_ok() {
                    let _ = status_tx.send("Playback stopped".to_string());
                    let _ = output.stop();
                    return;
                }

                if decode_handle.is_finished() {
                    match decode_handle.join() {
                        Ok(Ok(())) => {
                            break;
                        }
                        Ok(Err(e)) => {
                            let _ = status_tx.send(format!("Playback failed: {}", e));
                            let _ = output.stop();
                            return;
                        }
                        Err(_) => {
                            let _ =
                                status_tx.send("Playback failed: decode thread panicked".to_string());
                            let _ = output.stop();
                            return;
                        }
                    }
                }

                std::thread::sleep(Duration::from_millis(5));
            }

            for _ in 0..150 {
                if stop_rx.try_recv().is_ok() {
                    let _ = status_tx.send("Playback stopped".to_string());
                    let _ = output.stop();
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            if let Err(e) = output.stop() {
                let _ = status_tx.send(format!("Playback stop warning: {}", e));
            }

            let _ = status_tx.send("Playback finished".to_string());
        });

        self.playback_thread_handle = Some(handle);
        self.playback_state = PlaybackState::Playing;
        self.play_status = Some("Playback starting...".to_string());
    }

    pub(crate) fn signal_playback_stop(&mut self) {
        if let Some(tx) = self.playback_cancel_sender.take() {
            let _ = tx.send(());
        }
    }

    pub(crate) fn poll_playback_status(&mut self) {
        if let Some(rx) = &self.playback_status_receiver {
            let mut playback_ended = false;
            let mut playback_failed = false;
            let mut playback_finished = false;

            while let Ok(status) = rx.try_recv() {
                if status.starts_with("Playback running") {
                    self.playback_state = PlaybackState::Playing;
                } else if let Some(device_name) = status.strip_prefix("Playback device: ") {
                    if device_name == "<default>" {
                        self.active_output_device_name = audio::cpal_output::default_device_name();
                    } else {
                        self.active_output_device_name = Some(device_name.to_string());
                    }
                } else if status == "Playback finished" {
                    self.playback_state = PlaybackState::Stopped;
                    self.playback_start_instant = None;
                    playback_ended = true;
                    playback_finished = true;
                } else if status == "Playback stopped" {
                    if self.playback_state != PlaybackState::Paused {
                        self.playback_state = PlaybackState::Stopped;
                    }
                    self.playback_start_instant = None;
                    playback_ended = true;
                } else if status.starts_with("Playback failed") {
                    self.playback_state = PlaybackState::Stopped;
                    self.playback_start_instant = None;
                    playback_ended = true;
                    playback_failed = true;
                }
                self.play_status = Some(status);
            }

            if playback_ended {
                self.detach_playback_thread();
                self.playback_cancel_sender = None;
                self.playback_status_receiver = None;
                self.active_output_device_name = None;

                if self.pending_play_request {
                    self.pending_play_request = false;
                    self.request_play();
                    return;
                }

                if playback_failed
                    && (self.current_queue_index.is_some() || self.selected_queue_index.is_some())
                {
                    self.play_status = Some("Playback failed, skipping to next queued track".to_string());
                    self.play_next();
                    return;
                }

                if playback_finished
                    && (self.current_queue_index.is_some() || self.selected_queue_index.is_some())
                {
                    self.play_status = Some("Track finished, advancing to next queued track".to_string());
                    self.play_next();
                    return;
                }
            }
        }

        if self.playback_state == PlaybackState::Playing
            && !self.seek_interacting
            && self.pending_seek_ms.is_none()
        {
            self.update_playback_position_from_clock();
        }

        self.flush_pending_seek();
    }

    pub(crate) fn monitor_default_output_device_hotplug(&mut self) {
        if self.selected.is_some() {
            return;
        }
        if self.pending_play_request {
            return;
        }
        if !matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
        ) {
            return;
        }

        let now = Instant::now();
        if let Some(last) = self.last_default_device_poll_at {
            if now.duration_since(last) < Duration::from_millis(500) {
                return;
            }
        }
        self.last_default_device_poll_at = Some(now);

        let current_default = audio::cpal_output::default_device_name();
        if self.last_seen_default_output_device_name.is_none() {
            self.last_seen_default_output_device_name = current_default;
            return;
        }

        if self.last_seen_default_output_device_name != current_default {
            self.last_seen_default_output_device_name = current_default.clone();

            if self.active_output_device_name != current_default {
                self.playback_state = PlaybackState::Buffering;
                self.play_status = Some("Default output device changed, reconnecting...".to_string());
                self.request_play();
            }
        }
    }

    pub(crate) fn detach_playback_thread(&mut self) {
        let _ = self.playback_thread_handle.take();
    }

    pub(crate) fn update_playback_position_from_clock(&mut self) {
        if let Some(start_instant) = self.playback_start_instant {
            let elapsed = start_instant.elapsed();
            let elapsed_ms = elapsed.as_millis() as u64;
            let position = self.playback_start_ms.saturating_add(elapsed_ms);
            if let Some(duration) = self.play_duration_ms {
                self.play_position_ms = position.min(duration);
            } else {
                self.play_position_ms = position;
            }
        }
    }

    pub(crate) fn refresh_play_metadata(&mut self) {
        let path = PathBuf::from(self.play_path.clone());
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_duration_ms = None;
            self.play_title = None;
            self.play_artist = None;
            self.play_album = None;
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        let decoder = SymphoniaDecoder::new();
        if let Ok(decoded) = decoder.decode_file(&path) {
            let frames = decoded.frames() as u64;
            let sample_rate = decoded.sample_rate as u64;
            if sample_rate > 0 {
                self.play_duration_ms = Some(frames.saturating_mul(1000) / sample_rate);
                if let Some(duration_ms) = self.play_duration_ms {
                    self.play_position_ms = self.play_position_ms.min(duration_ms);
                }
            }
            self.play_title = decoded.title;
            self.play_artist = decoded.artist;
            self.play_album = decoded.album;
        }
    }
}
