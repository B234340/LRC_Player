use super::overlay_time_ms;
use lrc_player_application::infrastructure::lyrics::{save_lrc_offset, LyricsTimeline};

#[test]
fn user_offset_is_applied_exactly_once() {
    let mut timeline = LyricsTimeline::parse("[00:01.000]A\n[00:02.000]B\n");
    timeline.set_user_offset_ms(500);
    // Positive offset = lyrics earlier: at 1.6 s playback the lyric clock is 2.1 s.
    assert_eq!(overlay_time_ms(Some(&timeline), 1_600), 2_100);
    assert_eq!(timeline.line_index_at(1_600), Some(1));
}

#[test]
fn negative_offset_clamps_at_zero() {
    let mut timeline = LyricsTimeline::parse("[00:01.000]A\n");
    timeline.set_user_offset_ms(-2_000);
    assert_eq!(overlay_time_ms(Some(&timeline), 500), 0);
    assert_eq!(overlay_time_ms(None, 500), 500);
}

#[test]
fn file_offset_tag_and_user_offset_add_once() {
    let mut timeline = LyricsTimeline::parse("[offset:+200]\n[00:01.000]A\n");
    timeline.set_user_offset_ms(100);
    assert_eq!(overlay_time_ms(Some(&timeline), 1_000), 1_300);
}

#[test]
fn saving_offset_keeps_display_identical_after_reload() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("song.lrc");
    std::fs::write(&path, "[offset:+200]\n[00:01.000]A\n[00:03.000]B\n").unwrap();

    let mut before = LyricsTimeline::from_lrc(&path).unwrap();
    before.set_user_offset_ms(-700);
    let probes = [0u64, 900, 1_500, 2_400, 3_600];
    let expected: Vec<_> = probes.iter().map(|&p| before.line_index_at(p)).collect();

    let saved = save_lrc_offset(&path, -700).unwrap();
    assert!(saved.backup_path.is_some());

    // After saving, the user offset is reset to 0 and the file carries the shift.
    let after = LyricsTimeline::from_lrc(&path).unwrap();
    assert_eq!(after.user_offset_ms(), 0);
    assert_eq!(after.file_offset_ms(), 0);
    let actual: Vec<_> = probes.iter().map(|&p| after.line_index_at(p)).collect();
    assert_eq!(actual, expected);
}
