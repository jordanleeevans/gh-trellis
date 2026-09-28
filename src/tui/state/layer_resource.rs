use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LayerResource<T> {
    Loading,
    Ready(T),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LayerResourceCache<T> {
    entries: HashMap<String, LayerResource<T>>,
}

impl<T> Default for LayerResourceCache<T> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<T> LayerResourceCache<T> {
    pub(crate) fn get(&self, key: &str) -> Option<&T> {
        match self.entries.get(key) {
            Some(LayerResource::Ready(value)) => Some(value),
            _ => None,
        }
    }

    pub(crate) fn is_loading(&self, key: &str) -> bool {
        matches!(self.entries.get(key), Some(LayerResource::Loading))
    }

    pub(crate) fn should_load(&self, key: &str, force: bool) -> bool {
        match self.entries.get(key) {
            Some(LayerResource::Loading) => false,
            Some(LayerResource::Ready(_)) => force,
            Some(LayerResource::Failed(_)) | None => true,
        }
    }

    pub(crate) fn mark_loading(&mut self, key: String) {
        self.entries.insert(key, LayerResource::Loading);
    }

    pub(crate) fn store_result(&mut self, key: String, result: Result<T, String>) {
        let resource = match result {
            Ok(value) => LayerResource::Ready(value),
            Err(error) => LayerResource::Failed(error),
        };
        self.entries.insert(key, resource);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracks_loading_ready_and_force_reload() {
        let mut cache = LayerResourceCache::default();
        let key = "stack::layer".to_string();

        assert!(cache.should_load(&key, false));
        cache.mark_loading(key.clone());
        assert!(cache.is_loading(&key));
        assert!(!cache.should_load(&key, true));

        cache.store_result(key.clone(), Ok("diff".to_string()));
        assert_eq!(cache.get(&key).map(String::as_str), Some("diff"));
        assert!(!cache.is_loading(&key));
        assert!(!cache.should_load(&key, false));
        assert!(cache.should_load(&key, true));
    }
}
