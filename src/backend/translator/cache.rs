//! 翻訳キャッシュ型定義

use dashmap::DashMap;
use std::sync::Mutex;

/// セッション中のLLM翻訳結果キャッシュ（揮発・スレッドセーフ）
pub type TranslationCache = DashMap<String, String>;

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
