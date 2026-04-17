use anyhow::Result;
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::meta::MetadataOptions;
use symphonia::default::get_probe;

use crate::infrastructure::decode::DecodedSource;

#[derive(Default)]
pub struct SymphoniaDecoder;

impl SymphoniaDecoder {
    pub fn new() -> Self {
        Self
    }

    pub fn decode_file<P: AsRef<Path>>(&self, path: P) -> Result<DecodedSource> {
        let path = path.as_ref();
        let src = File::open(path)?;
        let mss = symphonia::core::io::MediaSourceStream::new(Box::new(src), Default::default());
        let probe = get_probe().format(
            &Default::default(),
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;
        let mut format = probe.format;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.sample_rate.is_some())
            .ok_or_else(|| anyhow::anyhow!("no audio track found"))?;
        let codec_params = &track.codec_params;
        let sample_rate = codec_params
            .sample_rate
            .ok_or_else(|| anyhow::anyhow!("sample rate unknown"))?
            as u32;
        let channels = codec_params
            .channels
            .as_ref()
            .map(|c| c.count() as u16)
            .unwrap_or(2);

        let mut decoder =
            symphonia::default::get_codecs().make(codec_params, &DecoderOptions::default())?;
        let mut samples: Vec<f32> = Vec::new();

        while let Ok(packet) = format.next_packet() {
            match decoder.decode(&packet) {
                Ok(audio_buf) => {
                    let frames = audio_buf.frames();
                    let mut sb = SampleBuffer::<f32>::new(
                        frames as symphonia::core::units::Duration,
                        *audio_buf.spec(),
                    );
                    sb.copy_interleaved_ref(audio_buf);
                    samples.extend_from_slice(sb.samples());
                }
                Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
                Err(e) => return Err(anyhow::anyhow!(e.to_string())),
            }
        }

        let mut title = None;
        let mut artist = None;
        let mut album = None;

        if let Some(metadata) = format.metadata().current() {
            for tag in metadata.tags() {
                if let symphonia::core::meta::Value::String(value) = &tag.value {
                    let key = tag.key.to_lowercase();
                    if key == "title" {
                        let _ = title.get_or_insert_with(|| value.clone());
                    } else if key == "artist" {
                        let _ = artist.get_or_insert_with(|| value.clone());
                    } else if key == "album" {
                        let _ = album.get_or_insert_with(|| value.clone());
                    }
                }
            }
        }

        Ok(DecodedSource::with_metadata(
            samples,
            sample_rate,
            channels,
            title,
            artist,
            album,
        ))
    }
}


