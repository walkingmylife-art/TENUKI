//! 翻訳キャッシュ型定義

use dashmap::DashMap;
use std::sync::Mutex;

/// セッション中のLLM翻訳結果キャッシュ（揮発・スレッドセーフ）
#[derive(Default)]
pub struct TranslationCache {
    source_index: DashMap<String, String>,
    value_index: DashMap<String, String>,
}

impl TranslationCache {
    pub fn insert(&self, key: String, value: String) {
        self.source_index.insert(key, value.clone());

        // Value observation index: raw translated value -> same raw value.
        // This is not source authority and must not normalize or trim.
        if !value.is_empty() {
            self.value_index.entry(value.clone()).or_insert(value);
        }
    }

    pub fn lookup_source(&self, key: &str) -> Option<String> {
        self.source_index.get(key).map(|v| v.clone())
    }

    /// Raw value observation lookup. This is not a source lookup.
    pub fn lookup_value(&self, text: &str) -> Option<String> {
        self.value_index.get(text).map(|v| v.clone())
    }

    #[cfg(test)]
    pub fn get(&self, key: &str) -> Option<String> {
        self.lookup_source(key)
    }

    pub fn clear(&self) {
        self.source_index.clear();
        self.value_index.clear();
    }
}

/// シャットダウン時に辞書へ書き込む新規翻訳エントリ（挿入順保持・スレッドセーフ）
#[derive(Default)]
pub struct NewEntriesCache {
    inner: Mutex<Vec<(String, String)>>,
}

impl NewEntriesCache {
    /// キーが未登録の場合のみ末尾に追加する（先勝ち）
    pub fn insert(&self, key: String, value: String) {
        let mut v = self.inner.lock().unwrap();
        if !v.iter().any(|(k, _)| k == &key) {
            v.push((key, value));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }

    /// 全エントリを挿入順で取り出し、内部をクリアする
    pub fn drain(&self) -> Vec<(String, String)> {
        std::mem::take(&mut *self.inner.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_cache_source_lookup_uses_raw_key() {
        let cache = TranslationCache::default();
        cache.insert("ATK ".to_string(), " Value ".to_string());

        assert_eq!(cache.lookup_source("ATK "), Some(" Value ".to_string()));
        assert_eq!(cache.lookup_source("ATK"), None);
        assert_eq!(cache.lookup_source("atk "), None);
    }

    #[test]
    fn translation_cache_value_lookup_uses_raw_value() {
        let cache = TranslationCache::default();
        cache.insert("source".to_string(), " Value ".to_string());

        assert_eq!(cache.lookup_value(" Value "), Some(" Value ".to_string()));
        assert_eq!(cache.lookup_value("Value"), None);
        assert_eq!(cache.lookup_value(" value "), None);
    }
}
