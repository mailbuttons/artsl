//! The service's durable state: art tokens and daily usage, in one redb file
//! next to the pictures. Pure Rust, single file, survives restarts, needs no
//! database server. One process owns it at a time.

use anyhow::{Context, Result};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// sha256(token) hex -> JSON `TokenRecord`
const TOKENS: TableDefinition<&str, &[u8]> = TableDefinition::new("tokens");
/// "{sha256(token) hex}:{YYYY-MM-DD}" -> pictures drawn that day
const USAGE: TableDefinition<&str, u32> = TableDefinition::new("usage");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenRecord {
    pub created_at: String,
    pub last_used_at: Option<String>,
    /// Free text the client sent at subscribe time, e.g. a hostname.
    pub label: String,
    pub revoked: bool,
    pub pictures: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenSummary {
    pub hash_prefix: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
    pub label: String,
    pub revoked: bool,
    pub pictures: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub tokens: u64,
    pub active_tokens: u64,
    pub pictures_total: u64,
    pub pictures_today: u64,
    pub tokens_used_today: u64,
}

pub struct Store {
    db: Database,
}

impl Store {
    pub fn open(path: &Path) -> Result<Store> {
        let db = Database::create(path).with_context(|| format!("opening {}", path.display()))?;
        let txn = db.begin_write()?;
        {
            txn.open_table(TOKENS)?;
            txn.open_table(USAGE)?;
        }
        txn.commit()?;
        Ok(Store { db })
    }

    pub fn insert_token(&self, hash: &str, rec: &TokenRecord) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(TOKENS)?;
            t.insert(hash, serde_json::to_vec(rec)?.as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    pub fn get_token(&self, hash: &str) -> Result<Option<TokenRecord>> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(TOKENS)?;
        Ok(match t.get(hash)? {
            Some(v) => Some(serde_json::from_slice(v.value())?),
            None => None,
        })
    }

    /// Mark a token revoked. Returns whether it existed and was active.
    pub fn revoke(&self, hash: &str) -> Result<bool> {
        let txn = self.db.begin_write()?;
        let changed = {
            let mut t = txn.open_table(TOKENS)?;
            let existing: Option<TokenRecord> = match t.get(hash)? {
                Some(v) => Some(serde_json::from_slice(v.value())?),
                None => None,
            };
            match existing {
                Some(mut rec) if !rec.revoked => {
                    rec.revoked = true;
                    t.insert(hash, serde_json::to_vec(&rec)?.as_slice())?;
                    true
                }
                _ => false,
            }
        };
        txn.commit()?;
        Ok(changed)
    }

    /// Revoke every active token whose hash starts with `prefix` (at least 8
    /// characters, to make an accidental sweep impossible). Returns how many.
    pub fn revoke_by_prefix(&self, prefix: &str) -> Result<usize> {
        if prefix.len() < 8 {
            anyhow::bail!("prefix too short");
        }
        let hashes: Vec<String> = {
            let txn = self.db.begin_read()?;
            let t = txn.open_table(TOKENS)?;
            t.iter()?
                .filter_map(|r| r.ok())
                .map(|(k, _)| k.value().to_string())
                .filter(|k| k.starts_with(prefix))
                .collect()
        };
        let mut n = 0;
        for h in hashes {
            if self.revoke(&h)? {
                n += 1;
            }
        }
        Ok(n)
    }

    /// Count one picture against `hash` for `day`, unless `limit` is already
    /// reached. Atomic: one write transaction.
    pub fn take_quota(
        &self,
        hash: &str,
        day: &str,
        limit: u32,
    ) -> Result<std::result::Result<u32, u32>> {
        let key = format!("{hash}:{day}");
        let txn = self.db.begin_write()?;
        let outcome = {
            let mut t = txn.open_table(USAGE)?;
            let used = t.get(key.as_str())?.map(|v| v.value()).unwrap_or(0);
            if used >= limit {
                Err(limit)
            } else {
                t.insert(key.as_str(), used + 1)?;
                Ok(used + 1)
            }
        };
        txn.commit()?;
        Ok(outcome)
    }

    pub fn note_picture(&self, hash: &str, now: &str) -> Result<()> {
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(TOKENS)?;
            let rec: Option<TokenRecord> = match t.get(hash)? {
                Some(v) => Some(serde_json::from_slice(v.value())?),
                None => None,
            };
            if let Some(mut rec) = rec {
                rec.last_used_at = Some(now.to_string());
                rec.pictures += 1;
                t.insert(hash, serde_json::to_vec(&rec)?.as_slice())?;
            }
        }
        txn.commit()?;
        Ok(())
    }

    /// Drop usage rows for days before `keep_from` (YYYY-MM-DD, lexical order).
    pub fn purge_usage_before(&self, keep_from: &str) -> Result<usize> {
        let old: Vec<String> = {
            let txn = self.db.begin_read()?;
            let t = txn.open_table(USAGE)?;
            t.iter()?
                .filter_map(|r| r.ok())
                .map(|(k, _)| k.value().to_string())
                .filter(|k| {
                    k.rsplit_once(':')
                        .map(|(_, d)| d < keep_from)
                        .unwrap_or(true)
                })
                .collect()
        };
        if old.is_empty() {
            return Ok(0);
        }
        let txn = self.db.begin_write()?;
        {
            let mut t = txn.open_table(USAGE)?;
            for k in &old {
                t.remove(k.as_str())?;
            }
        }
        txn.commit()?;
        Ok(old.len())
    }

    pub fn stats(&self, today: &str) -> Result<Stats> {
        let txn = self.db.begin_read()?;
        let tokens = txn.open_table(TOKENS)?;
        let usage = txn.open_table(USAGE)?;
        let mut active = 0u64;
        let mut pictures_total = 0u64;
        for r in tokens.iter()? {
            let (_, v) = r?;
            let rec: TokenRecord = serde_json::from_slice(v.value())?;
            if !rec.revoked {
                active += 1;
            }
            pictures_total += rec.pictures;
        }
        let suffix = format!(":{today}");
        let mut pictures_today = 0u64;
        let mut tokens_used_today = 0u64;
        for r in usage.iter()? {
            let (k, v) = r?;
            if k.value().ends_with(&suffix) {
                pictures_today += v.value() as u64;
                tokens_used_today += 1;
            }
        }
        Ok(Stats {
            tokens: tokens.len()?,
            active_tokens: active,
            pictures_total,
            pictures_today,
            tokens_used_today,
        })
    }

    pub fn list_tokens(&self, limit: usize) -> Result<Vec<TokenSummary>> {
        let txn = self.db.begin_read()?;
        let t = txn.open_table(TOKENS)?;
        let mut out = Vec::new();
        for r in t.iter()? {
            let (k, v) = r?;
            let rec: TokenRecord = serde_json::from_slice(v.value())?;
            out.push(TokenSummary {
                hash_prefix: k.value().chars().take(12).collect(),
                created_at: rec.created_at,
                last_used_at: rec.last_used_at,
                label: rec.label,
                revoked: rec.revoked,
                pictures: rec.pictures,
            });
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out.truncate(limit);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("t.redb")).unwrap();
        (dir, store)
    }

    fn rec() -> TokenRecord {
        TokenRecord {
            created_at: "2026-10-29T00:00:00Z".into(),
            last_used_at: None,
            label: "laptop".into(),
            revoked: false,
            pictures: 0,
        }
    }

    #[test]
    fn tokens_round_trip_and_revoke() {
        let (_d, s) = fresh();
        s.insert_token("abcdef0123456789", &rec()).unwrap();
        assert_eq!(
            s.get_token("abcdef0123456789").unwrap().unwrap().label,
            "laptop"
        );
        assert!(s.get_token("nope").unwrap().is_none());
        assert!(s.revoke("abcdef0123456789").unwrap());
        assert!(!s.revoke("abcdef0123456789").unwrap());
        assert!(s.get_token("abcdef0123456789").unwrap().unwrap().revoked);
        assert!(s.revoke_by_prefix("abc").is_err());
    }

    #[test]
    fn quota_counts_per_day() {
        let (_d, s) = fresh();
        assert_eq!(s.take_quota("h", "2026-10-29", 2).unwrap(), Ok(1));
        assert_eq!(s.take_quota("h", "2026-10-29", 2).unwrap(), Ok(2));
        assert_eq!(s.take_quota("h", "2026-10-29", 2).unwrap(), Err(2));
        assert_eq!(s.take_quota("h", "2026-10-30", 2).unwrap(), Ok(1));
        assert_eq!(s.purge_usage_before("2026-10-30").unwrap(), 1);
        assert_eq!(s.take_quota("h", "2026-10-29", 2).unwrap(), Ok(1));
    }

    #[test]
    fn stats_add_up() {
        let (_d, s) = fresh();
        s.insert_token("a".repeat(16).as_str(), &rec()).unwrap();
        s.insert_token("b".repeat(16).as_str(), &rec()).unwrap();
        s.revoke(&"b".repeat(16)).unwrap();
        s.take_quota(&"a".repeat(16), "2026-10-29", 5)
            .unwrap()
            .unwrap();
        s.note_picture(&"a".repeat(16), "2026-10-29T01:00:00Z")
            .unwrap();
        let st = s.stats("2026-10-29").unwrap();
        assert_eq!(
            (
                st.tokens,
                st.active_tokens,
                st.pictures_total,
                st.pictures_today,
                st.tokens_used_today
            ),
            (2, 1, 1, 1, 1)
        );
        let list = s.list_tokens(10).unwrap();
        assert_eq!(list.len(), 2);
    }
}
