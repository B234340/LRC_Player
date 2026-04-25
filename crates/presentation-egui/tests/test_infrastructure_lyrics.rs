#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use lrc_player::infrastructure::lyrics::*;
    use tempfile::tempdir;

    #[test]
    fn loads_timestamp_formats_from_lrc() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("song.lrc");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "[00:00.00]Intro").unwrap();
        writeln!(file, "[01:02.345]Chorus").unwrap();
        writeln!(file, "[10:20.5]Outro").unwrap();

        let lyrics = Lyrics::from_lrc(&path).unwrap();
        assert_eq!(lyrics.lines.len(), 3);
        assert_eq!(lyrics.lines[0].time_ms, 0);
        assert_eq!(lyrics.lines[1].time_ms, 62_345);
        assert_eq!(lyrics.lines[2].time_ms, 620_500);
    }

    #[test]
    fn loads_lrc_contents() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("song.lrc");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "[00:00.00]Intro").unwrap();
        writeln!(file, "[00:10.50]Verse 1").unwrap();

        let lyrics = Lyrics::from_lrc(&path).unwrap();
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[0].text, "Intro");
        assert_eq!(lyrics.lines[1].time_ms, 10_500);
    }

    #[test]
    fn discovers_lrc_in_same_folder() {
        let dir = tempdir().unwrap();
        let track = dir.path().join("track.mp3");
        File::create(&track).unwrap();
        let lyric = dir.path().join("track.lrc");
        File::create(&lyric).unwrap();

        let found = Lyrics::discover_for_track(&track, &[]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap(), "track.lrc");
    }

    #[test]
    fn parses_enhanced_lrc_without_displaying_timestamps() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("song.lrc");
        let mut file = File::create(&path).unwrap();
        writeln!(file, "[00:00.033]<00:00.033>world.<00:00.547>execute (<00:00.576>me) ; <00:00.608>[<00:00.640>Key <00:00.671>Ingredient <00:00.702>ver.] <00:00.733>- <00:00.743><00:00.733>Mili<00:00.779>").unwrap();

        let lyrics = Lyrics::from_lrc(&path).unwrap();
        assert_eq!(lyrics.lines.len(), 2);
        assert_eq!(lyrics.lines[1].text, "world.execute (me) ; [Key Ingredient ver.] - Mili");
        assert!(!lyrics.lines[1].text.contains("<00"));
        assert!(lyrics.lines[1].segments.len() > 1);
        assert_eq!(lyrics.lines[1].segments[0].time_ms, 33);
        assert_eq!(lyrics.lines[1].segments[1].time_ms, 33);
        assert_eq!(lyrics.lines[1].segments[2].time_ms, 547);
    }
}
