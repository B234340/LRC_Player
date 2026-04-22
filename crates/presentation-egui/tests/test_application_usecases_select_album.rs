#[cfg(test)]
mod tests {
    use lrc_player::application::usecases::select_album::*;

    #[test]
    fn selects_album_by_id() {
        let selected = execute("album-123");
        assert_eq!(selected, "album-123");
    }
}