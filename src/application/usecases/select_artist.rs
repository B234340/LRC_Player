/// Select an artist by returning the selected artist identifier.
pub fn execute(artist_id: &str) -> String {
    artist_id.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_artist_by_id() {
        let selected = execute("artist-123");
        assert_eq!(selected, "artist-123");
    }
}
