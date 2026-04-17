#[cfg(test)]
mod tests {
    use lrc_player::infrastructure::audio::stream_adapter::StreamConverter;

    #[test]
    fn process_preserves_frame_count_for_same_rate_same_channels() {
        let mut conv = StreamConverter::new(44100, 2, 44100, 2, 32);
        let samples: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let mut out = Vec::new();
        conv.process(&samples, 2, 44100, &mut out);

        assert_eq!(out.len(), samples.len());
        assert_eq!(out, samples);
    }

    #[test]
    fn process_preserves_frame_count_for_mono_to_stereo() {
        let mut conv = StreamConverter::new(44100, 1, 44100, 2, 32);
        let samples: Vec<f32> = vec![0.0, 0.5, -0.5, 1.0];
        let mut out = Vec::new();
        conv.process(&samples, 1, 44100, &mut out);

        assert_eq!(out.len(), samples.len() * 2);
        assert_eq!(out, vec![0.0, 0.0, 0.5, 0.5, -0.5, -0.5, 1.0, 1.0]);
    }

    #[test]
    fn process_resamples_down_by_half_at_same_position() {
        let mut conv = StreamConverter::new(44100, 1, 22050, 1, 32);
        let samples: Vec<f32> = (0..8).map(|i| i as f32).collect();
        let mut out = Vec::new();
        conv.process(&samples, 1, 44100, &mut out);

        assert!(out.len() < samples.len());
        assert_eq!(out.len(), 4);
    }
}