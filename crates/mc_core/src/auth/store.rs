//! Persistent account storage (`accounts.json`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::auth::Account;
use crate::error::Result;
use crate::util::{read_json_or_default, write_json};

#[derive(Debug, Default, Serialize, Deserialize)]
struct AccountsFile {
    #[serde(default)]
    active: Option<String>,
    #[serde(default)]
    accounts: Vec<Account>,
}

/// A small, crash-safe account repository backed by a JSON file.
#[derive(Debug)]
pub struct AccountStore {
    path: PathBuf,
    data: AccountsFile,
}

impl AccountStore {
    /// Load the store from `path`, treating a missing file as empty.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        let data: AccountsFile = read_json_or_default(&path).await?;
        Ok(Self { path, data })
    }

    pub fn accounts(&self) -> &[Account] {
        &self.data.accounts
    }

    pub fn active_id(&self) -> Option<&str> {
        self.data.active.as_deref()
    }

    pub fn active(&self) -> Option<&Account> {
        let id = self.data.active.as_deref()?;
        self.data.accounts.iter().find(|a| a.id == id)
    }

    /// Insert or replace an account by id, then persist.
    pub async fn upsert(&mut self, account: Account) -> Result<()> {
        if let Some(existing) = self.data.accounts.iter_mut().find(|a| a.id == account.id) {
            *existing = account;
        } else {
            self.data.accounts.push(account);
        }
        if self.data.active.is_none() {
            self.data.active = self.data.accounts.first().map(|a| a.id.clone());
        }
        self.save().await
    }

    /// Remove an account and pick a new active account if needed.
    pub async fn remove(&mut self, id: &str) -> Result<()> {
        self.data.accounts.retain(|a| a.id != id);
        if self.data.active.as_deref() == Some(id) {
            self.data.active = self.data.accounts.first().map(|a| a.id.clone());
        }
        self.save().await
    }

    /// Mark an account as active.
    pub async fn set_active(&mut self, id: &str) -> Result<()> {
        if self.data.accounts.iter().any(|a| a.id == id) {
            self.data.active = Some(id.to_string());
            self.save().await?;
        }
        Ok(())
    }

    /// Persist the current state to disk.
    pub async fn save(&self) -> Result<()> {
        write_json(&self.path, &self.data).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn round_trip_accounts() {
        let dir = std::env::temp_dir().join(format!("ctm-test-{}", uuid::Uuid::new_v4()));
        let path = dir.join("accounts.json");

        let mut store = AccountStore::load(&path).await.unwrap();
        assert!(store.accounts().is_empty());

        store.upsert(Account::offline("Steve")).await.unwrap();
        store.upsert(Account::offline("Alex")).await.unwrap();
        assert_eq!(store.accounts().len(), 2);
        assert_eq!(store.active().unwrap().username, "Steve");

        let alex_id = store.accounts()[1].id.clone();
        store.set_active(&alex_id).await.unwrap();

        let reloaded = AccountStore::load(&path).await.unwrap();
        assert_eq!(reloaded.active().unwrap().username, "Alex");

        let mut reloaded = reloaded;
        reloaded.remove(&alex_id).await.unwrap();
        assert_eq!(reloaded.active().unwrap().username, "Steve");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
