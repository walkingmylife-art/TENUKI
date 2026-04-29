//! 翻訳キャッシュ型定義

use dashmap::DashMap;
use std::sync::Mutex;

use crate::backend::normalize::normalize_key;

/// セッション中のLLM翻訳結果キャッシュ（揮発・スレッドセーフ）
#[derive(Default)]
pub struct TranslationCache {
    source_index: DashMap<String, String>,
    value_index: DashMap<String, String>,
}

impl TranslationCache {
    pub fn insert(&self, key: String, value: String) {
        let normalized_key = normalize_key(key.trim());
        let normalized_value = normalize_key(value.trim());

        self.source_index.insert(normalized_key, value.clone());

        if !normalized_value.is_empty() {
            self.value_index.entry(normalized_value).or_insert(value);
        }
    }

    pub fn lookup_source(&self, key: &str) -> Option<String> {
        let normalized = normalize_key(key.trim());
        self.source_index.get(&normalized).map(|v| v.clone())
    }

    pub fn lookup_value(&self, text: &str) -> Option<String> {
        let normalized = normalize_key(text.trim());
        self.value_index.get(&normalized).map(|v| v.clone())
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
