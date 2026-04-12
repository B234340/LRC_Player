//! Audio 出力周りのコア実装
//!
//! - `BufferedAudioOutput` はアプリ側のオーディオフレームブロックを保持する有界キューを提供します。
//! - `AudioDeviceOutput` は `cpal` を用いて OS の出力ストリームを構築し、`BufferedAudioOutput` からサンプルを取り出して
//!   デバイスへ書き込みます。
//!
//! 設計上、バッファはブロック単位で enqueue/dequeue され、デバイススレッドはできるだけ短時間で次のサンプルを取得
//! できるように最小限のコピーで処理します。

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use crossbeam_queue::ArrayQueue;

/// デバイス／バッファ設定
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioOutputConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub frames_per_buffer: u32,
}

impl Default for AudioOutputConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48_000,
            channels: 2,
            frames_per_buffer: 1_024,
        }
    }
}

/// フレームブロック表現（チャンネル数・フレーム数・平坦化された f32 サンプル配列）
#[derive(Debug, Clone, PartialEq)]
pub struct AudioFrameBlock {
    pub channels: u16,
    pub frames: u32,
    pub samples: Box<[f32]>,
}

impl AudioFrameBlock {
    /// 指定長の無音ブロックを生成するユーティリティ
    pub fn silence(channels: u16, frames: u32) -> Self {
        let sample_count = channels as usize * frames as usize;
        Self {
            channels,
            frames,
            samples: vec![0.0; sample_count].into_boxed_slice(),
        }
    }

    /// ブロックの長さをミリ秒で計算
    pub fn duration_ms(&self, sample_rate: u32) -> u32 {
        if sample_rate == 0 {
            return 0;
        }

        ((self.frames as u64 * 1_000) / sample_rate as u64) as u32
    }
}

/// アプリ側のバッファを表すハイレベルラッパ
#[derive(Clone)]
pub struct BufferedAudioOutput {
    /// 出力設定（サンプルレート、チャンネル数、フレーム長）
    config: AudioOutputConfig,
    /// 内部ブロックキュー（bounded）
    queue: Arc<ArrayQueue<AudioFrameBlock>>,
    /// 再生がアクティブかどうか（デバイススレッド参照用）
    playback_active: Arc<AtomicBool>,
    /// 現在キューに溜まっているサンプル数の近似値
    pending_samples: Arc<AtomicUsize>,
    /// バッファクリア時に増える epoch（デバイス側で検出して未消化サンプルを調整）
    reset_epoch: Arc<AtomicU64>,
}

impl BufferedAudioOutput {
    pub fn new(config: AudioOutputConfig, block_capacity: usize) -> Self {
        Self {
            config,
            queue: Arc::new(ArrayQueue::new(block_capacity)),
            playback_active: Arc::new(AtomicBool::new(false)),
            pending_samples: Arc::new(AtomicUsize::new(0)),
            reset_epoch: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn config(&self) -> AudioOutputConfig {
        self.config
    }

    pub fn clear(&self) {
        while let Some(block) = self.queue.pop() {
            self.pending_samples
                .fetch_sub(block.samples.len(), Ordering::Relaxed);
        }
        self.reset_epoch.fetch_add(1, Ordering::Relaxed);
    }

    pub fn try_enqueue(&self, block: AudioFrameBlock) -> Result<(), AudioFrameBlock> {
        let sample_count = block.samples.len();
        match self.queue.push(block) {
            Ok(()) => {
                self.pending_samples
                    .fetch_add(sample_count, Ordering::Relaxed);
                Ok(())
            }
            Err(block) => Err(block),
        }
    }

    pub fn try_enqueue_many<I>(&self, blocks: I) -> usize
    where
        I: IntoIterator<Item = AudioFrameBlock>,
    {
        let mut inserted = 0;
        for block in blocks {
            if self.try_enqueue(block).is_err() {
                break;
            }
            inserted += 1;
        }
        inserted
    }

    pub fn pop_or_silence(&self) -> AudioFrameBlock {
        // キューからブロックを取り出す。なければ無音ブロックを返す。
        // Try to pop a block from the bounded queue. When a block is obtained,
        // decrement `pending_samples` to keep the approximate pending-samples metric aligned.
        // This is approximate because `pending_samples` accounts for sample count rather than
        // exact frame bookkeeping.
        if let Some(block) = self.queue.pop() {
            self.pending_samples
                .fetch_sub(block.samples.len(), Ordering::Relaxed);
            return block;
        }

        // If the queue is empty, return a short silence block so the device always receives data.
        AudioFrameBlock::silence(self.config.channels, self.config.frames_per_buffer)
    }

    pub fn buffered_blocks(&self) -> usize {
        self.queue.len()
    }

    pub fn block_capacity(&self) -> usize {
        self.queue.capacity()
    }

    pub fn buffered_ms(&self) -> u32 {
        // pending_samples はサンプル数を示すため、それをチャンネル数で割りフレーム数にし、
        // サンプルレートからミリ秒へ換算する。
        // Convert approximate pending sample count to milliseconds:
        // frames = pending_samples / channels
        // ms = frames * 1000 / sample_rate
        ((self.pending_samples.load(Ordering::Relaxed) as u64 / self.config.channels.max(1) as u64
            * 1_000)
            / self.config.sample_rate as u64) as u32
    }

    pub fn is_full(&self) -> bool {
        self.queue.is_full()
    }

    pub fn set_playback_active(&self, active: bool) {
        self.playback_active.store(active, Ordering::Relaxed);
    }

    pub fn pending_samples(&self) -> usize {
        self.pending_samples.load(Ordering::Relaxed)
    }

    fn shared_queue(&self) -> Arc<ArrayQueue<AudioFrameBlock>> {
        Arc::clone(&self.queue)
    }

    fn shared_playback_active(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.playback_active)
    }

    fn shared_pending_samples(&self) -> Arc<AtomicUsize> {
        Arc::clone(&self.pending_samples)
    }

    fn shared_reset_epoch(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.reset_epoch)
    }
}

pub struct AudioDeviceOutput {
    _stream: Stream,
}

impl AudioDeviceOutput {
    /// Query the preferred (default) device configuration, if available
    pub fn preferred_config() -> Option<AudioOutputConfig> {
        let host = cpal::default_host();
        let device = host.default_output_device()?;
        let supported = device.default_output_config().ok()?;
        let config = supported.config();
        Some(AudioOutputConfig {
            sample_rate: config.sample_rate.0,
            channels: config.channels,
            frames_per_buffer: match config.buffer_size {
                BufferSize::Fixed(frames) => frames,
                BufferSize::Default => 1_024,
            },
        })
    }

    pub fn config_for_name(device_name: &str) -> Option<AudioOutputConfig> {
        let host = cpal::default_host();
        let devices = host.output_devices().ok()?;
        for device in devices {
            if let Ok(name) = device.name() {
                if name == device_name {
                    if let Ok(supported) = device.default_output_config() {
                        let cfg = supported.config();
                        return Some(AudioOutputConfig {
                            sample_rate: cfg.sample_rate.0,
                            channels: cfg.channels,
                            frames_per_buffer: match cfg.buffer_size {
                                BufferSize::Fixed(frames) => frames,
                                BufferSize::Default => 1_024,
                            },
                        });
                    }
                }
            }
        }
        None
    }

    pub fn try_start(buffered_output: &BufferedAudioOutput) -> Result<Self, String> {
        // start default output device and hook into buffered_output
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .ok_or_else(|| "no default output device available".to_string())?;
        let supported = device
            .default_output_config()
            .map_err(|error| format!("failed to query default output config: {error}"))?;
        let stream_config = supported.config();
        let queue = buffered_output.shared_queue();
        let playback_active = buffered_output.shared_playback_active();
        let pending_samples = buffered_output.shared_pending_samples();
        let reset_epoch = buffered_output.shared_reset_epoch();

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_output_stream::<f32>(
                &device,
                &stream_config,
                queue,
                playback_active,
                pending_samples,
                reset_epoch,
            ),
            SampleFormat::I16 => build_output_stream::<i16>(
                &device,
                &stream_config,
                queue,
                playback_active,
                pending_samples,
                reset_epoch,
            ),
            SampleFormat::U16 => build_output_stream::<u16>(
                &device,
                &stream_config,
                queue,
                playback_active,
                pending_samples,
                reset_epoch,
            ),
            other => Err(format!("unsupported output sample format: {other:?}")),
        }?;

        stream
            .play()
            .map_err(|error| format!("failed to start output stream: {error}"))?;

        Ok(Self { _stream: stream })
    }

    pub fn list_output_devices() -> Vec<String> {
        let host = cpal::default_host();
        match host.output_devices() {
            Ok(devices) => devices.filter_map(|d| d.name().ok()).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn try_start_by_name(
        buffered_output: &BufferedAudioOutput,
        device_name: &str,
    ) -> Result<Self, String> {
        let host = cpal::default_host();
        let mut selected = None;

        let devices = host
            .output_devices()
            .map_err(|e| format!("failed to enumerate output devices: {e}"))?;

        for device in devices {
            if let Ok(name) = device.name() {
                if name == device_name {
                    selected = Some(device);
                    break;
                }
            }
        }

        let device = selected.ok_or_else(|| format!("device '{}' not found", device_name))?;

        let supported = device
            .default_output_config()
            .map_err(|error| format!("failed to query default output config: {error}"))?;
        let stream_config = supported.config();
        let queue = buffered_output.shared_queue();
        let playback_active = buffered_output.shared_playback_active();
        let pending_samples = buffered_output.shared_pending_samples();
        let reset_epoch = buffered_output.shared_reset_epoch();

        let stream = match supported.sample_format() {
            SampleFormat::F32 => build_output_stream::<f32>(
                &device,
                &stream_config,
                queue,
                playback_active,
                pending_samples,
                reset_epoch,
            ),
            SampleFormat::I16 => build_output_stream::<i16>(
                &device,
                &stream_config,
                queue,
                playback_active,
                pending_samples,
                reset_epoch,
            ),
            SampleFormat::U16 => build_output_stream::<u16>(
                &device,
                &stream_config,
                queue,
                playback_active,
                pending_samples,
                reset_epoch,
            ),
            other => Err(format!("unsupported output sample format: {other:?}")),
        }?;

        stream
            .play()
            .map_err(|error| format!("failed to start output stream: {error}"))?;

        Ok(Self { _stream: stream })
    }
}

fn build_output_stream<T>(
    device: &cpal::Device,
    stream_config: &StreamConfig,
    queue: Arc<ArrayQueue<AudioFrameBlock>>,
    playback_active: Arc<AtomicBool>,
    pending_samples: Arc<AtomicUsize>,
    reset_epoch: Arc<AtomicU64>,
) -> Result<Stream, String>
where
    T: Sample + SizedSample + FromSample<f32>,
{
    // `cpal` の出力ストリームを生成するユーティリティ。
    // クロージャ内で `write_output_data` を呼び、BufferedAudioOutput からサンプルを取得して書き込む。
    let channels = stream_config.channels as usize;
    let err_playback_active = Arc::clone(&playback_active);
    let mut current_block: Option<AudioFrameBlock> = None;
    let mut current_sample_index = 0usize;
    let mut seen_reset_epoch = reset_epoch.load(Ordering::Relaxed);

    device
        .build_output_stream(
            stream_config,
            move |data: &mut [T], _| {
                write_output_data(
                    data,
                    channels,
                    &queue,
                    &playback_active,
                    &pending_samples,
                    &reset_epoch,
                    &mut current_block,
                    &mut current_sample_index,
                    &mut seen_reset_epoch,
                );
            },
            move |error| {
                err_playback_active.store(false, Ordering::Relaxed);
                eprintln!("audio output stream error: {error}");
            },
            None,
        )
        .map_err(|error| format!("failed to build output stream: {error}"))
}

fn write_output_data<T>(
    data: &mut [T],
    channels: usize,
    queue: &ArrayQueue<AudioFrameBlock>,
    playback_active: &AtomicBool,
    pending_samples: &AtomicUsize,
    reset_epoch: &AtomicU64,
    current_block: &mut Option<AudioFrameBlock>,
    current_sample_index: &mut usize,
    seen_reset_epoch: &mut u64,
) where
    T: Sample + FromSample<f32>,
{
    // Core device write loop:
    // - Use `reset_epoch` to detect when the main thread cleared/reset the buffer; drop
    //   any partially used current block when epochs differ.
    // - If playback is inactive write silence; otherwise consume samples from current blocks.
    // - Convert f32 samples into target sample type `T` and write into the `output` slice.
    let current_epoch = reset_epoch.load(Ordering::Relaxed);
    if *seen_reset_epoch != current_epoch {
        if let Some(block) = current_block.as_ref() {
            let remaining = block.samples.len().saturating_sub(*current_sample_index);
            if remaining > 0 {
                pending_samples.fetch_sub(remaining, Ordering::Relaxed);
            }
        }
        *current_block = None;
        *current_sample_index = 0;
        *seen_reset_epoch = current_epoch;
    }

    if !playback_active.load(Ordering::Relaxed) {
        for sample in data.iter_mut() {
            *sample = T::from_sample(0.0);
        }
        return;
    }

    for sample in data.iter_mut() {
        let next = next_output_sample(
            queue,
            pending_samples,
            current_block,
            current_sample_index,
            channels,
        );
        *sample = T::from_sample(next);
    }
}

fn next_output_sample(
    queue: &ArrayQueue<AudioFrameBlock>,
    pending_samples: &AtomicUsize,
    current_block: &mut Option<AudioFrameBlock>,
    current_sample_index: &mut usize,
    output_channels: usize,
) -> f32 {
    loop {
        if let Some(block) = current_block.as_ref() {
            if *current_sample_index < block.samples.len() {
                let sample = block.samples[*current_sample_index];
                *current_sample_index += 1;
                pending_samples.fetch_sub(1, Ordering::Relaxed);
                return sample;
            }
        }

        if let Some(block) = queue.pop() {
            *current_sample_index = 0;
            *current_block = Some(remap_channels(block, output_channels));
            continue;
        }

        *current_block = None;
        *current_sample_index = 0;
        return 0.0;
    }
}

fn remap_channels(block: AudioFrameBlock, output_channels: usize) -> AudioFrameBlock {
    let input_channels = block.channels as usize;
    if input_channels == output_channels || input_channels == 0 || output_channels == 0 {
        return block;
    }

    let frames = block.frames as usize;
    let mut remapped = Vec::with_capacity(frames * output_channels);
    for frame_index in 0..frames {
        let frame_start = frame_index * input_channels;
        if input_channels > output_channels {
            remapped.extend_from_slice(&block.samples[frame_start..frame_start + output_channels]);
        } else {
            let mono = block.samples[frame_start];
            for _ in 0..output_channels {
                remapped.push(mono);
            }
        }
    }

    AudioFrameBlock {
        channels: output_channels as u16,
        frames: block.frames,
        samples: remapped.into_boxed_slice(),
    }
}

#[cfg(test)]
mod tests {
    use super::{AudioFrameBlock, AudioOutputConfig, BufferedAudioOutput};

    #[test]
    fn buffered_output_is_bounded() {
        let output = BufferedAudioOutput::new(AudioOutputConfig::default(), 2);

        assert!(output.try_enqueue(AudioFrameBlock::silence(2, 16)).is_ok());
        assert!(output.try_enqueue(AudioFrameBlock::silence(2, 16)).is_ok());
        assert!(output.try_enqueue(AudioFrameBlock::silence(2, 16)).is_err());
        assert_eq!(output.buffered_blocks(), 2);
        assert!(output.is_full());
    }
}
