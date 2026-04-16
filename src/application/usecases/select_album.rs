/// Select an album by returning the selected album identifier.
pub fn execute(album_id: &str) -> String {
    album_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_album_by_id() {
        let selected = execute("album-123");
        assert_eq!(selected, "album-123");
    }
}
