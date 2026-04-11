use std::collections::HashMap;

use crate::core::ids::StrId;

#[derive(Debug, Default, Clone)]
pub struct StringInterner {
    ids_by_value: HashMap<String, StrId>,
    values: Vec<String>,
}

impl StringInterner {
    pub fn intern<S: Into<String>>(&mut self, value: S) -> StrId {
        let value = value.into();
        if let Some(existing) = self.ids_by_value.get(&value) {
            return *existing;
        }

        let id = StrId(self.values.len() as u32);
        self.values.push(value.clone());
        self.ids_by_value.insert(value, id);
        id
    }

    pub fn resolve(&self, id: StrId) -> Option<String> {
        self.values.get(id.0 as usize).cloned()
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}
