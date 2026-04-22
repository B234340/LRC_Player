#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use lrc_player::infrastructure::persistence::json_store::*;
    use tempfile::tempdir;

    #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
    struct TestData {
        name: String,
        value: i32,
    }

    #[test]
    fn saves_and_loads_json_roundtrip() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.json");
        let store = JsonStore::open(&path).unwrap();

        let original = TestData {
            name: "hello".into(),
            value: 42,
        };
        store.save(&original).unwrap();

        let loaded: TestData = store.load().unwrap();
        assert_eq!(loaded, original);
    }
}
