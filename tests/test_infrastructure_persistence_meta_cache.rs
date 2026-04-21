#[cfg(test)]
mod tests {
    use lrc_player::infrastructure::persistence::meta_cache::*;

    #[test]
    fn init_in_memory() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");
        // schema should exist
        let row: i64 = cache
            .connection()
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |r| r.get(0),
            )
            .expect("query sqlite_master");
        assert!(row >= 1);
    }

    #[test]
    fn upsert_staging_and_checkpoint_moves_to_main() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");

        // insert into staging
        cache
            .upsert_file_staging(
                "/tmp/song1.mp3",
                "/tmp/song1.mp3",
                1234,
                42,
                None,
                None,
                None,
            )
            .expect("upsert staging");

        // ensure staging has 1 row
        let staging_count: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files_staging", [], |r| r.get(0))
            .expect("count staging");
        assert_eq!(staging_count, 1);

        // ensure main files table is empty
        let main_count: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .expect("count files");
        assert_eq!(main_count, 0);

        // checkpoint should move staging -> main and clear staging
        cache.checkpoint().expect("checkpoint");

        let main_count_after: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files", [], |r| r.get(0))
            .expect("count files after");
        assert_eq!(main_count_after, 1);

        let staging_count_after: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM files_staging", [], |r| r.get(0))
            .expect("count staging after");
        assert_eq!(staging_count_after, 0);

        // get_file_id should return Some
        let id = cache.get_file_id("/tmp/song1.mp3").expect("get id");
        assert!(id.is_some());
    }

    #[test]
    fn upsert_file_and_get_file_id() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");
        cache
            .upsert_file(
                "/tmp/song2.mp3",
                "/tmp/song2.mp3",
                4321,
                43,
                None,
                None,
                None,
            )
            .expect("upsert file");
        let id = cache.get_file_id("/tmp/song2.mp3").expect("get id");
        assert!(id.is_some());
    }

    #[test]
    fn checkpoint_tracks_locations_hashes() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");

        cache
            .upsert_track_staging(
                "track-1",
                Some("Song 1"),
                Some("Artist"),
                None,
                None,
                Some("Album"),
                None,
                100,
            )
            .expect("upsert track staging");
        cache
            .upsert_location_staging(
                "track-1",
                "/tmp/song1.mp3",
                None,
                None,
                1234,
                42,
                1620000000,
                true,
                true,
            )
            .expect("upsert location staging");
        cache
            .upsert_hash_staging("track-1", "sha1", Some("abcdef"), Some("abcd"), 1620000000)
            .expect("upsert hash staging");

        cache.checkpoint().expect("checkpoint");

        let track_count: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM tracks", [], |r| r.get(0))
            .expect("count tracks");
        assert_eq!(track_count, 1);

        let location_count: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM locations", [], |r| r.get(0))
            .expect("count locations");
        assert_eq!(location_count, 1);

        let hash_count: i64 = cache
            .connection()
            .query_row("SELECT count(*) FROM hashes", [], |r| r.get(0))
            .expect("count hashes");
        assert_eq!(hash_count, 1);
    }

    #[test]
    fn library_counts_and_recent_locations_work() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");

        cache
            .upsert_track(
                "track-a",
                Some("Song A"),
                Some("Artist A"),
                None,
                None,
                Some("Album X"),
                None,
                100,
            )
            .expect("upsert track A");
        cache
            .upsert_track(
                "track-b",
                Some("Song B"),
                Some("Artist B"),
                None,
                None,
                Some("Album X"),
                None,
                101,
            )
            .expect("upsert track B");
        cache
            .upsert_track(
                "track-c",
                Some("Song C"),
                Some("Artist C"),
                None,
                None,
                Some("Album Y"),
                None,
                102,
            )
            .expect("upsert track C");

        cache
            .upsert_location(
                "track-a",
                "/tmp/track-a.mp3",
                None,
                None,
                1234,
                42,
                1620000000,
                false,
                true,
            )
            .expect("upsert location A");
        cache
            .upsert_location(
                "track-b",
                "/tmp/track-b.mp3",
                None,
                None,
                1235,
                43,
                1620000001,
                false,
                true,
            )
            .expect("upsert location B");
        cache
            .upsert_location(
                "track-c",
                "/tmp/track-c.mp3",
                None,
                None,
                1236,
                44,
                1620000002,
                false,
                true,
            )
            .expect("upsert location C");

        let (album_count, track_count) = cache.library_counts().expect("library counts");
        assert_eq!(album_count, 2);
        assert_eq!(track_count, 3);

        let recent = cache.recent_locations(2).expect("recent locations");
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0], "/tmp/track-c.mp3");
        assert_eq!(recent[1], "/tmp/track-b.mp3");
    }

    #[test]
    fn duplicate_groups_returns_same_stable_key_paths() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");
        cache
            .upsert_file(
                "/tmp/song1.mp3",
                "/tmp/song1.mp3",
                1000,
                1,
                None,
                None,
                Some("stable-a"),
            )
            .expect("upsert file 1");
        cache
            .upsert_file(
                "/tmp/song2.mp3",
                "/tmp/song2.mp3",
                1000,
                1,
                None,
                None,
                Some("stable-a"),
            )
            .expect("upsert file 2");
        cache
            .upsert_file(
                "/tmp/song3.mp3",
                "/tmp/song3.mp3",
                1000,
                1,
                None,
                None,
                Some("stable-b"),
            )
            .expect("upsert file 3");

        let groups = cache.duplicate_groups().expect("duplicate groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].0, "stable-a");
        assert_eq!(groups[0].1, vec!["/tmp/song1.mp3".to_string(), "/tmp/song2.mp3".to_string()]);
    }

    #[test]
    fn library_album_and_artist_lists_work() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");

        cache
            .upsert_track(
                "track-a",
                Some("Song A"),
                Some("Artist B"),
                None,
                None,
                Some("Album X"),
                None,
                100,
            )
            .expect("upsert track A");
        cache
            .upsert_track(
                "track-b",
                Some("Song B"),
                Some("Artist A"),
                None,
                None,
                Some("Album Y"),
                None,
                101,
            )
            .expect("upsert track B");
        cache
            .upsert_track(
                "track-c",
                Some("Song C"),
                Some("Artist B"),
                None,
                None,
                Some("Album X"),
                None,
                102,
            )
            .expect("upsert track C");

        let albums = cache.library_albums().expect("library albums");
        assert_eq!(albums, vec!["Album X".to_string(), "Album Y".to_string()]);

        let artists = cache.library_artists().expect("library artists");
        assert_eq!(artists, vec!["Artist A".to_string(), "Artist B".to_string()]);
    }

    #[test]
    fn library_track_queries_by_album_and_artist_work() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");

        cache
            .upsert_track(
                "track-a",
                Some("Song A"),
                Some("Artist A"),
                None,
                None,
                Some("Album X"),
                None,
                100,
            )
            .expect("upsert track A");
        cache
            .upsert_track(
                "track-b",
                Some("Song B"),
                Some("Artist A"),
                None,
                None,
                Some("Album Y"),
                None,
                101,
            )
            .expect("upsert track B");
        cache
            .upsert_track(
                "track-c",
                Some("Song C"),
                Some("Artist B"),
                None,
                None,
                Some("Album X"),
                None,
                102,
            )
            .expect("upsert track C");

        let album_x_tracks = cache
            .library_tracks_by_album("Album X")
            .expect("album X tracks");
        assert_eq!(album_x_tracks, vec!["Song A".to_string(), "Song C".to_string()]);

        let artist_a_tracks = cache
            .library_tracks_by_artist("Artist A")
            .expect("artist A tracks");
        assert_eq!(artist_a_tracks, vec!["Song A".to_string(), "Song B".to_string()]);
    }

    #[test]
    fn recent_directories_work() {
        let cache = MetaCache::new(":memory:").expect("create in-memory db");

        cache
            .upsert_track(
                "track-a",
                Some("Song A"),
                Some("Artist A"),
                None,
                None,
                Some("Album A"),
                None,
                100,
            )
            .expect("upsert track A");
        cache
            .upsert_location(
                "track-a",
                "/tmp/music/Artist A/Album A/track-a.mp3",
                None,
                None,
                1234,
                42,
                1620000000,
                true,
                true,
            )
            .expect("upsert location A");
        cache
            .upsert_track(
                "track-b",
                Some("Song B"),
                Some("Artist B"),
                None,
                None,
                Some("Album B"),
                None,
                101,
            )
            .expect("upsert track B");
        cache
            .upsert_location(
                "track-b",
                "/tmp/music/Artist B/Album B/track-b.mp3",
                None,
                None,
                1235,
                43,
                1620000001,
                true,
                true,
            )
            .expect("upsert location B");

        let dirs = cache.recent_directories(2).expect("recent directories");
        assert_eq!(dirs.len(), 2);
        assert_eq!(dirs[0], "/tmp/music/Artist B/Album B");
        assert_eq!(dirs[1], "/tmp/music/Artist A/Album A");
    }
}
