#[cfg(test)]
mod tests {
    use lrc_player::application::usecases::select_artist::*;

    #[test]
    fn selects_artist_by_id() {
        let selected = execute("artist-123");
        assert_eq!(selected, "artist-123");
    }
}