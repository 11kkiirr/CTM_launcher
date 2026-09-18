//! Authentication: Microsoft OAuth2 Device Code Flow and offline accounts.

pub mod microsoft;
pub mod offline;
pub mod store;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use microsoft::{DeviceCodePrompt, MicrosoftAuth};
pub use offline::offline_uuid;
pub use store::AccountStore;

/// The origin of an account's session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AccountKind {
    /// A real Microsoft/Xbox account authenticated through OAuth2.
    Microsoft,
    /// A local, unauthenticated profile usable in singleplayer/LAN.
    Offline,
}

/// A launchable player account.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// The player's UUID (dashed, lowercase).
    pub id: String,
    /// Display name / in-game username.
    pub username: String,
    pub kind: AccountKind,
    /// Minecraft Services access token (`None` for offline accounts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    /// OAuth2 refresh token, used to renew `access_token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Expiry for `access_token`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Xbox user hash, required when building the identity token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub xuid: Option<String>,
    /// Cached URL of the account's current skin.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin_url: Option<String>,
}

impl Account {
    /// Build an offline account from a username.
    pub fn offline(username: impl Into<String>) -> Self {
        let username = username.into();
        let id = offline_uuid(&username);
        Self {
            id,
            username,
            kind: AccountKind::Offline,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            xuid: None,
            skin_url: None,
        }
    }

    /// Whether the stored token is still valid for at least `skew_secs`.
    pub fn token_valid(&self, skew_secs: i64) -> bool {
        match self.expires_at {
            Some(exp) => exp > Utc::now() + chrono::Duration::seconds(skew_secs),
            None => false,
        }
    }

    /// The identity token used by the `--accessToken` launch argument.
    ///
    /// Offline accounts use a deterministic placeholder token.
    pub fn launch_token(&self) -> String {
        match (&self.kind, &self.access_token) {
            (AccountKind::Microsoft, Some(tok)) => tok.clone(),
            _ => "0".to_string(),
        }
    }
}
