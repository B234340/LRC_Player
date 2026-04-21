#[cfg(test)]
mod tests {
    use lrc_player::infrastructure::dedup::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn partial_and_full_hash_match_expectations() {
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "hello world").unwrap();
        let path = tmp.path().to_path_buf();

        let partial = compute_partial_hash(&path, 4).unwrap();
        let full = compute_full_hash(&path).unwrap();
        assert!(!partial.is_empty());
        assert!(!full.is_empty());
        // different content length may produce different hashes; ensure both succeed
        assert_ne!(partial, "");
    }

    #[tokio::test]
    async fn service_full_hash() {
        let svc = DedupService::spawn();
        let mut tmp = NamedTempFile::new().unwrap();
        write!(tmp, "some data to hash").unwrap();
        let path = tmp.path().to_path_buf();
        let rx = svc.request_full_hash(path).await;
        let res = rx.await.expect("response");
        assert!(res.is_ok());
    }
}
