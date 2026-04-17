#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::io::Write;
    use std::path::PathBuf;
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

        let found = Lyrics::discover_for_track(&track);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].file_name().unwrap(), "track.lrc");
    }
}
