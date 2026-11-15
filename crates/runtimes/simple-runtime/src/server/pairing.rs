use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use parking_lot::Mutex;
use subtle::ConstantTimeEq;

/// 已签发 Token 的持久化集合。Token 一经签发长期有效，与之后的 Pairing Key 无关。
pub struct TokenStore {
    path: PathBuf,
    tokens: Mutex<HashSet<String>>,
}

impl TokenStore {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        let path = dir.join("tokens.json");
        let tokens = if path.exists() {
            let data = fs::read_to_string(&path)?;
            serde_json::from_str(&data).unwrap_or_default()
        } else {
            HashSet::new()
        };
        Ok(Self {
            path,
            tokens: Mutex::new(tokens),
        })
    }

    pub fn contains(&self, token: &str) -> bool {
        self.tokens.lock().contains(token)
    }

    /// 签发并落盘一个新 Token。落盘失败时不留下内存中的 Token，避免重启后失效的配对。
    pub fn issue(&self) -> std::io::Result<String> {
        let bytes: [u8; 32] = rand::random();
        let token = bytes.iter().map(|b| format!("{b:02x}")).collect::<String>();
        let mut g = self.tokens.lock();
        g.insert(token.clone());
        if let Err(e) = self.persist(&g) {
            g.remove(&token);
            return Err(e);
        }
        Ok(token)
    }

    /// 先写临时文件再改名，崩溃时不会留下截断的 tokens.json 而让所有设备失去配对。
    fn persist(&self, tokens: &HashSet<String>) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(tokens).unwrap_or_else(|_| b"[]".to_vec());
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, data)?;
        fs::rename(&tmp, &self.path)
    }
}

/// 常量时间比较 Pairing Key。空 Key 视为未设置，永不匹配。
pub fn key_matches(expected: &str, provided: &str) -> bool {
    !expected.is_empty() && bool::from(expected.as_bytes().ct_eq(provided.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir() -> PathBuf {
        std::env::temp_dir().join(format!("it-pair-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn key_matches_exact_only() {
        assert!(key_matches("sam01101", "sam01101"));
        assert!(!key_matches("sam01101", "sam01102"));
        assert!(!key_matches("sam01101", "sam0110"));
        assert!(!key_matches("sam01101", "sam011011"));
        assert!(!key_matches("sam01101", ""));
    }

    #[test]
    fn empty_expected_key_never_matches() {
        assert!(!key_matches("", ""));
        assert!(!key_matches("", "anything"));
    }

    #[test]
    fn issued_token_is_64_hex_and_survives_reopen() {
        let dir = tmp_dir();
        let store = TokenStore::open(&dir).unwrap();
        let token = store.issue().unwrap();
        assert_eq!(token.len(), 64);
        assert!(token.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(store.contains(&token));
        drop(store);
        let reopened = TokenStore::open(&dir).unwrap();
        assert!(reopened.contains(&token));
        assert!(!dir.join("tokens.json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn two_tokens_are_distinct_and_both_kept() {
        let dir = tmp_dir();
        let store = TokenStore::open(&dir).unwrap();
        let a = store.issue().unwrap();
        let b = store.issue().unwrap();
        assert_ne!(a, b);
        let reopened = TokenStore::open(&dir).unwrap();
        assert!(reopened.contains(&a) && reopened.contains(&b));
        fs::remove_dir_all(dir).unwrap();
    }
}
