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
    /// 再生を要求します。
    /// 
    /// この関数は以下の処理を行います：
    /// 1. 再生パスの有効性チェック（空でない、存在する）
    /// 2. 既存の再生スレッドがある場合は停止要求を送信し、保留フラグを設定
    /// 3. 新しい再生スレッドを起動
    ///    - オーディオ出力デバイスの初期化
    ///    - デコードスレッドの起動
    ///    - ステータス監視ループの実行
    /// 
    /// 再生スレッドは独立して動作し、ステータスは`poll_playback_status()`で
    /// 定期的にポーリングされます。
    /// 
    /// # 注意
    /// - シーク中の場合は`playback_state`が`Seeking`のまま維持されます
    /// - シーク中でない場合は`Buffering`状態に遷移します
    pub(crate) fn request_play(&mut self) {
        let path = PathBuf::from(self.play_path.clone());

        // 再生パスの有効性チェック
        if path.as_os_str().is_empty() {
            self.play_status = Some("Play path is empty.".to_string());
            return;
        }
        if !path.exists() {
            self.play_status = Some(format!("Play path does not exist: {}", path.display()));
            return;
        }

        // シーク操作中かどうかを記録
        let is_seeking = self.playback_state == PlaybackState::Seeking;

        // 既存の再生スレッドがある場合
        if self.playback_thread_handle.is_some() {
            // 停止信号を送信
            self.signal_playback_stop();
            // 保留フラグを設定（スレッド終了後に再要求される）
            self.pending_play_request = true;
            if !is_seeking {
                self.playback_state = PlaybackState::Buffering;
            }
            self.play_status = Some("Stopping previous playback...".to_string());
            return;
        }

        // 新しい再生を開始
        self.pending_play_request = false;
        if !is_seeking {
            self.playback_state = PlaybackState::Buffering;
        }
        self.play_status = Some(format!("Starting playback: {}", path.display()));
        
        // 再生開始時刻と開始位置を記録（経過時間計算用）
        self.playback_start_instant = Some(Instant::now());
        self.playback_start_ms = self.play_position_ms;
        self.save_state();

        let start_ms = Some(self.play_position_ms);

        // ステータス通信用チャネルを作成
        // - status_tx/rx: 再生スレッドからUIスレッドへのステータス通知
        // - stop_tx/rx: UIスレッドから再生スレッドへの停止要求
        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let (stop_tx, stop_rx) = crossbeam_channel::bounded::<()>(1);
        self.playback_status_receiver = Some(status_rx);
        self.playback_cancel_sender = Some(stop_tx);

        // ユーザーが選択した出力デバイス（未選択の場合はデフォルト）
        let preferred_device = self
            .selected
            .and_then(|idx| self.device_names.get(idx).cloned());
        
        // リサンプリング品質（タップ数）
        let taps = self.taps;

        // 再生スレッドを起動
        let handle = std::thread::spawn(move || {
            let mut output = audio::CpalOutput::new();

            // 要求するオーディオフォーマット
            let requested = AudioFormat {
                sample_rate: 44_100,
                channels: 2,
                sample_format: SampleFormat::F32,
            };

            // オーディオ出力を開始
            let actual = match output.start_with_device(requested, preferred_device) {
                Ok(fmt) => fmt,
                Err(e) => {
                    let _ = status_tx.send(format!("Playback failed to start output: {}", e));
                    return;
                }
            };

            // 実際に使用されたデバイス名を通知
            let selected_device_name = output.selected_device_name();
            if let Some(device_name) = selected_device_name {
                let _ = status_tx.send(format!("Playback device: {}", device_name));
            } else {
                let _ = status_tx.send("Playback device: <default>".to_string());
            }

            // オーディオデータを送信するためのプロデューサーを取得
            let producer_tx = match output.producer_sender() {
                Some(tx) => tx,
                None => {
                    let _ = status_tx.send("Playback failed: producer channel unavailable".to_string());
                    let _ = output.stop();
                    return;
                }
            };

            let pool = output.pool_handle();
            let chunk_frames = 2048; // 一度に処理するフレーム数

            let _ = status_tx.send(format!(
                "Playback running: {} Hz / {} ch",
                actual.sample_rate, actual.channels
            ));

            // デコードスレッドを起動
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

            // 再生監視ループ
            loop {
                // 停止要求をチェック
                if stop_rx.try_recv().is_ok() {
                    let _ = status_tx.send("Playback stopped".to_string());
                    let _ = output.stop();
                    return;
                }

                // デコード完了をチェック
                if decode_handle.is_finished() {
                    match decode_handle.join() {
                        Ok(Ok(())) => {
                            // 正常完了
                            break;
                        }
                        Ok(Err(e)) => {
                            // デコードエラー
                            let _ = status_tx.send(format!("Playback failed: {}", e));
                            let _ = output.stop();
                            return;
                        }
                        Err(_) => {
                            // スレッドパニック
                            let _ =
                                status_tx.send("Playback failed: decode thread panicked".to_string());
                            let _ = output.stop();
                            return;
                        }
                    }
                }

                // 短いスリープでCPU使用率を抑える
                std::thread::sleep(Duration::from_millis(5));
            }

            // バッファに残ったサンプルを排出するため少し待機
            // 最大150回（1.5秒）待機するが、停止要求があれば即座に終了
            for _ in 0..150 {
                if stop_rx.try_recv().is_ok() {
                    let _ = status_tx.send("Playback stopped".to_string());
                    let _ = output.stop();
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }

            // 出力を停止
            if let Err(e) = output.stop() {
                let _ = status_tx.send(format!("Playback stop warning: {}", e));
            }

            let _ = status_tx.send("Playback finished".to_string());
        });

        self.playback_thread_handle = Some(handle);
        self.playback_state = PlaybackState::Playing;
        self.play_status = Some("Playback starting...".to_string());
    }

    /// 再生バックエンドに停止信号を送信します。
    /// 
    /// チャネルを通じて停止要求を送信し、再生スレッドを停止させます。
    /// 送信後は`playback_cancel_sender`が`None`になります。
    /// 
    /// # 注意
    /// この関数は停止信号を送信するだけで、スレッドの終了を待機しません。
    /// 実際の終了処理は`poll_playback_status()`で検出されます。
    pub(crate) fn signal_playback_stop(&mut self) {
        if let Some(tx) = self.playback_cancel_sender.take() {
            let _ = tx.send(());
        }
    }

    /// 再生ステータスをポーリングします。
    /// 
    /// この関数は毎フレーム呼び出され、再生スレッドからのステータスメッセージを
    /// 受信して状態を更新します。
    /// 
    /// 処理内容：
    /// - ステータスメッセージの受信と解釈
    /// - 再生状態の更新
    /// - 再生完了時の後続処理（次のトラックへの自動遷移）
    /// - 保留中の再生要求の処理
    /// - 再生位置の更新（クロックベース）
    /// - 保留中のシーク要求の実行
    pub(crate) fn poll_playback_status(&mut self) {
        if let Some(rx) = &self.playback_status_receiver {
            let mut playback_ended = false;
            let mut playback_failed = false;
            let mut playback_finished = false;

            // キューに溜まったすべてのステータスメッセージを処理
            while let Ok(status) = rx.try_recv() {
                if status.starts_with("Playback running") {
                    // 再生が正常に開始された
                    self.playback_state = PlaybackState::Playing;
                } else if let Some(device_name) = status.strip_prefix("Playback device: ") {
                    // 使用中のデバイス名を記録
                    if device_name == "<default>" {
                        self.active_output_device_name = audio::cpal_output::default_device_name();
                    } else {
                        self.active_output_device_name = Some(device_name.to_string());
                    }
                } else if status == "Playback finished" {
                    // 正常に最後まで再生完了
                    self.playback_state = PlaybackState::Stopped;
                    self.playback_start_instant = None;
                    playback_ended = true;
                    playback_finished = true;
                } else if status == "Playback stopped" {
                    // ユーザー操作による停止
                    if self.playback_state != PlaybackState::Paused {
                        self.playback_state = PlaybackState::Stopped;
                    }
                    self.playback_start_instant = None;
                    playback_ended = true;
                } else if status.starts_with("Playback failed") {
                    // 再生エラー
                    self.playback_state = PlaybackState::Stopped;
                    self.playback_start_instant = None;
                    playback_ended = true;
                    playback_failed = true;
                }
                self.play_status = Some(status);
            }

            // 再生が終了した場合の後処理
            if playback_ended {
                self.detach_playback_thread();
                self.playback_cancel_sender = None;
                self.playback_status_receiver = None;
                self.active_output_device_name = None;

                // 保留中の再生要求があれば処理
                if self.pending_play_request {
                    self.pending_play_request = false;
                    self.request_play();
                    return;
                }

                // 再生失敗時は次のトラックへスキップ
                if playback_failed
                    && (self.current_queue_index.is_some() || self.selected_queue_index.is_some())
                {
                    self.play_status = Some("Playback failed, skipping to next queued track".to_string());
                    self.play_next();
                    return;
                }

                // 正常完了時は次のトラックへ
                if playback_finished
                    && (self.current_queue_index.is_some() || self.selected_queue_index.is_some())
                {
                    self.play_status = Some("Track finished, advancing to next queued track".to_string());
                    self.play_next();
                    return;
                }
            }
        }

        // 再生中かつシーク操作中でない場合、再生位置を更新
        if self.playback_state == PlaybackState::Playing
            && !self.seek_interacting
            && self.pending_seek_ms.is_none()
        {
            self.update_playback_position_from_clock();
        }

        // 保留中のシーク要求を実行
        self.flush_pending_seek();
    }

    /// デフォルト出力デバイスのホットプラグを監視します。
    /// 
    /// システムのデフォルト出力デバイスが変更された場合（例：Bluetoothヘッドフォンの接続）、
    /// 再生を自動的に再接続します。
    /// 
    /// 監視がスキップされる条件：
    /// - ユーザーが明示的にデバイスを選択している場合
    /// - 再生要求が保留中の場合
    /// - 再生中でない場合
    /// 
    /// ポーリング間隔は500msに制限されています。
    pub(crate) fn monitor_default_output_device_hotplug(&mut self) {
        // ユーザーが明示的にデバイスを選択している場合はスキップ
        if self.selected.is_some() {
            return;
        }
        // 再生要求が保留中の場合はスキップ
        if self.pending_play_request {
            return;
        }
        // 再生中でない場合はスキップ
        if !matches!(
            self.playback_state,
            PlaybackState::Playing | PlaybackState::Buffering | PlaybackState::Seeking
        ) {
            return;
        }

        // ポーリング間隔を500msに制限
        let now = Instant::now();
        if let Some(last) = self.last_default_device_poll_at {
            if now.duration_since(last) < Duration::from_millis(500) {
                return;
            }
        }
        self.last_default_device_poll_at = Some(now);

        // 現在のデフォルトデバイスを取得
        let current_default = audio::cpal_output::default_device_name();
        if self.last_seen_default_output_device_name.is_none() {
            self.last_seen_default_output_device_name = current_default;
            return;
        }

        // デバイスが変更された場合
        if self.last_seen_default_output_device_name != current_default {
            self.last_seen_default_output_device_name = current_default.clone();

            // 再生中のデバイスが新しいデフォルトと異なる場合、再接続
            if self.active_output_device_name != current_default {
                self.playback_state = PlaybackState::Buffering;
                self.play_status = Some("Default output device changed, reconnecting...".to_string());
                self.request_play();
            }
        }
    }

    /// 再生スレッドをデタッチします。
    /// 
    /// `JoinHandle`をドロップすることでスレッドをデタッチし、
    /// UIスレッドが再生スレッドの終了を待機してブロックされるのを防ぎます。
    /// 
    /// デタッチ後もスレッドはバックグラウンドで実行を継続しますが、
    /// 終了時のクリーンアップは行われなくなります。
    /// そのため、この関数は再生スレッドがすでに終了していることが
    /// 確認された後にのみ呼び出す必要があります。
    pub(crate) fn detach_playback_thread(&mut self) {
        let _ = self.playback_thread_handle.take();
    }

    /// 内部クロックから現在の再生位置を更新します。
    /// 
    /// `playback_start_instant`からの経過時間を計算し、
    /// 開始時のシーク位置（`playback_start_ms`）に加算して現在位置を求めます。
    /// 
    /// 総再生時間が既知の場合は、その値を超えないようにクランプされます。
    /// 
    /// # 注意
    /// この関数は再生位置の近似値を提供します。実際の再生位置とは
    /// わずかにずれる可能性があります。
    pub(crate) fn update_playback_position_from_clock(&mut self) {
        if let Some(start_instant) = self.playback_start_instant {
            // 開始からの経過時間をミリ秒で計算
            let elapsed = start_instant.elapsed();
            let elapsed_ms = elapsed.as_millis() as u64;
            
            // 開始位置 + 経過時間 = 現在位置
            let position = self.playback_start_ms.saturating_add(elapsed_ms);
            
            // 総再生時間が既知の場合はクランプ
            if let Some(duration) = self.play_duration_ms {
                self.play_position_ms = position.min(duration);
            } else {
                self.play_position_ms = position;
            }
        }
    }

    /// 現在の再生パスのメタデータを更新します。
    /// 
    /// Symphoniaデコーダを使用して、以下の情報をファイルから取得します：
    /// - 総再生時間（フレーム数 / サンプルレート）
    /// - タイトル
    /// - アーティスト名
    /// - アルバム名
    /// 
    /// ファイルが存在しない場合や読み込みに失敗した場合は、
    /// メタデータがクリアされます。
    pub(crate) fn refresh_play_metadata(&mut self) {
        let path = PathBuf::from(self.play_path.clone());
        
        // パスが無効な場合は全メタデータをクリア
        if path.as_os_str().is_empty() || !path.exists() {
            self.play_duration_ms = None;
            self.play_title = None;
            self.play_artist = None;
            self.play_album = None;
            self.play_artwork_handle = None;
            self.play_artwork_source = None;
            return;
        }

        // デコーダでファイルを解析
        let decoder = SymphoniaDecoder::new();
        if let Ok(decoded) = decoder.decode_file(&path) {
            let frames = decoded.frames() as u64;
            let sample_rate = decoded.sample_rate as u64;
            
            // 総再生時間を計算（フレーム数 / サンプルレート）
            if sample_rate > 0 {
                self.play_duration_ms = Some(frames.saturating_mul(1000) / sample_rate);
                
                // 現在の再生位置が総時間を超えないように調整
                if let Some(duration_ms) = self.play_duration_ms {
                    self.play_position_ms = self.play_position_ms.min(duration_ms);
                }
            }
            
            // メタデータを設定
            self.play_title = decoded.title;
            self.play_artist = decoded.artist;
            self.play_album = decoded.album;
        }
    }
}