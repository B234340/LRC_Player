#[cfg(test)]
mod tests {
    use lrc_player::domain::model::library::*;
    use lrc_player::domain::ids::TrackId;
    use lrc_player::domain::model::track::Track;

    #[test]
    fn library_add_dedupes_identical_track_ids() {
        let mut library = Library::new();
        let track = Track {
            id: TrackId("track-1".into()),
            title: "First".into(),
        };

        library.add_track(track.clone());
        library.add_track(track);

        assert_eq!(library.tracks.len(), 1);
    }
}
