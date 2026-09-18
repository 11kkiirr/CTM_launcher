//! Microsoft OAuth2 Device Code Flow → Xbox Live → XSTS → Minecraft Services.
//!
//! The flow is split into small, independently testable steps so the UI can
//! display the device-code prompt while the token poll runs in the background.

use std::time::Duration;

use chrono::{Duration as ChronoDuration, Utc};
use serde::Deserialize;

use crate::auth::{Account, AccountKind};
use crate::error::{CoreError, Result};

/// Public client id used by several open-source launchers for the consumers
/// tenant. Override with `MicrosoftAuth::with_client_id` if you register your
/// own Azure application.
pub const DEFAULT_CLIENT_ID: &str = "00000000402b5328";

const DEVICE_CODE_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/devicecode";
const TOKEN_URL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const XBL_AUTH_URL: &str = "https://user.auth.xboxlive.com/user/authenticate";
const XSTS_AUTH_URL: &str = "https://xsts.auth.xboxlive.com/xsts/authorize";
const MC_LOGIN_URL: &str = "https://api.minecraftservices.com/authentication/login_with_xbox";
const MC_PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// The scope required for Minecraft authentication plus refresh tokens.
pub const SCOPE: &str = "XboxLive.signin offline_access";

/// The prompt the user must act on during device-code authentication.
#[derive(Debug, Clone, Deserialize)]
pub struct DeviceCodePrompt {
    pub user_code: String,
    pub device_code: String,
    pub verification_uri: String,
    #[serde(default)]
    pub expires_in: u64,
    #[serde(default = "default_interval")]
    pub interval: u64,
    #[serde(default)]
    pub message: String,
}

fn default_interval() -> u64 {
    5
}

impl DeviceCodePrompt {
    /// Seconds until the device code expires.
    pub fn expires_in(&self) -> Duration {
        Duration::from_secs(self.expires_in)
    }
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_in: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct OAuthError {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct XboxResponse {
    #[serde(rename = "Token")]
    token: String,
    #[serde(rename = "DisplayClaims")]
    display_claims: DisplayClaims,
}

#[derive(Debug, Deserialize)]
struct DisplayClaims {
    xui: Vec<Xui>,
}

#[derive(Debug, Deserialize)]
struct Xui {
    uhs: String,
}

#[derive(Debug, Deserialize)]
struct XstsError {
    #[serde(rename = "XErr")]
    xerr: i64,
}

#[derive(Debug, Deserialize)]
struct MinecraftAuthResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// The Mojang profile returned by `minecraft/profile`.
#[derive(Debug, Clone, Deserialize)]
pub struct MinecraftProfile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub skins: Vec<ProfileSkin>,
    #[serde(default)]
    pub capes: Vec<serde_json::Value>,
}

/// One skin entry on a profile.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileSkin {
    pub id: String,
    pub state: String,
    pub url: String,
    #[serde(default)]
    pub variant: Option<String>,
}

impl MinecraftProfile {
    /// URL of the currently active skin, if any.
    pub fn active_skin(&self) -> Option<&str> {
        self.skins
            .iter()
            .find(|s| s.state == "ACTIVE")
            .map(|s| s.url.as_str())
    }
}

/// Driver for the Microsoft authentication chain.
#[derive(Debug, Clone)]
pub struct MicrosoftAuth {
    client: reqwest::Client,
    client_id: String,
}

impl MicrosoftAuth {
    /// Create a driver with the default public client id.
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            client_id: DEFAULT_CLIENT_ID.to_string(),
        }
    }

    /// Create a driver with a custom Azure application (client) id.
    pub fn with_client_id(client: reqwest::Client, client_id: impl Into<String>) -> Self {
        Self {
            client,
            client_id: client_id.into(),
        }
    }

    /// Step 1 — request a device code and user-facing prompt.
    pub async fn request_device_code(&self) -> Result<DeviceCodePrompt> {
        let params = [("client_id", self.client_id.as_str()), ("scope", SCOPE)];
        let response = self
            .client
            .post(DEVICE_CODE_URL)
            .form(&params)
            .send()
            .await?;

        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CoreError::Auth(format!(
                "device code request failed: {text}"
            )));
        }
        Ok(response.json::<DeviceCodePrompt>().await?)
    }

    /// Step 2 — poll the token endpoint until the user completes sign-in.
    ///
    /// Respects the server-provided `interval` and `slow_down` backoff, and
    /// returns an error once the device code expires.
    pub async fn poll_for_token(&self, prompt: &DeviceCodePrompt) -> Result<TokenResponse> {
        let deadline = tokio::time::Instant::now() + prompt.expires_in();
        let mut interval = prompt.interval.max(1);

        loop {
            if tokio::time::Instant::now() >= deadline {
                return Err(CoreError::Auth(
                    "device code expired before sign-in completed".into(),
                ));
            }
            tokio::time::sleep(Duration::from_secs(interval)).await;

            let params = [
                ("client_id", self.client_id.as_str()),
                ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
                ("device_code", prompt.device_code.as_str()),
            ];
            let response = self.client.post(TOKEN_URL).form(&params).send().await?;

            if response.status().is_success() {
                return Ok(response.json::<TokenResponse>().await?);
            }

            let err: OAuthError = response.json().await.unwrap_or(OAuthError {
                error: "unknown_error".into(),
                error_description: None,
            });
            match err.error.as_str() {
                "authorization_pending" => continue,
                "slow_down" => {
                    interval += 5;
                    continue;
                }
                other => {
                    return Err(CoreError::Auth(format!(
                        "{other}: {}",
                        err.error_description.unwrap_or_default()
                    )))
                }
            }
        }
    }

    /// Convenience: poll a device-code prompt to completion and perform the
    /// full Microsoft → Xbox → Minecraft exchange, returning a ready account.
    pub async fn login_device_code(&self, prompt: &DeviceCodePrompt) -> Result<Account> {
        let token = self.poll_for_token(prompt).await?;
        self.complete_login(&token.access_token, token.refresh_token, token.expires_in)
            .await
    }

    /// Refresh an expired Microsoft access token.
    pub async fn refresh(&self, refresh_token: &str) -> Result<TokenResponse> {
        let params = [
            ("client_id", self.client_id.as_str()),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("scope", SCOPE),
        ];
        let response = self.client.post(TOKEN_URL).form(&params).send().await?;
        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CoreError::Auth(format!("token refresh failed: {text}")));
        }
        Ok(response.json::<TokenResponse>().await?)
    }

    /// Full Microsoft → Xbox → Minecraft exchange, producing a ready account.
    pub async fn complete_login(
        &self,
        ms_access_token: &str,
        refresh_token: Option<String>,
        expires_in: Option<u64>,
    ) -> Result<Account> {
        let (xbl_token, uhs) = self.xbox_live_authenticate(ms_access_token).await?;
        let xsts_token = self.xsts_authorize(&xbl_token, &uhs).await?;
        let (mc_token, mc_expires) = self.minecraft_login(&uhs, &xsts_token).await?;
        let profile = self.fetch_profile(&mc_token).await?;

        let expires_at =
            Some(Utc::now() + ChronoDuration::seconds(mc_expires.unwrap_or(86_400) as i64));
        let _ = expires_in;
        let skin_url = profile.active_skin().map(str::to_string);

        Ok(Account {
            id: profile.id,
            username: profile.name,
            kind: AccountKind::Microsoft,
            access_token: Some(mc_token),
            refresh_token,
            expires_at,
            xuid: Some(uhs),
            skin_url,
        })
    }

    /// Renew an account's Minecraft session using its stored refresh token.
    pub async fn refresh_account(&self, account: &Account) -> Result<Account> {
        let refresh_token = account
            .refresh_token
            .as_deref()
            .ok_or_else(|| CoreError::Auth("account has no refresh token".into()))?;
        let token = self.refresh(refresh_token).await?;
        self.complete_login(
            &token.access_token,
            token
                .refresh_token
                .or_else(|| account.refresh_token.clone()),
            token.expires_in,
        )
        .await
    }

    async fn xbox_live_authenticate(&self, ms_access_token: &str) -> Result<(String, String)> {
        let body = serde_json::json!({
            "Properties": {
                "AuthMethod": "RPS",
                "SiteName": "user.auth.xboxlive.com",
                "RpsTicket": format!("d={ms_access_token}")
            },
            "RelyingParty": "http://auth.xboxlive.com",
            "TokenType": "JWT"
        });
        let response = self
            .client
            .post(XBL_AUTH_URL)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        let parsed: XboxResponse = response.json().await?;
        let uhs = parsed
            .display_claims
            .xui
            .into_iter()
            .next()
            .map(|x| x.uhs)
            .ok_or_else(|| CoreError::Auth("Xbox Live response missing user hash".into()))?;
        Ok((parsed.token, uhs))
    }

    async fn xsts_authorize(&self, xbl_token: &str, _uhs: &str) -> Result<String> {
        let body = serde_json::json!({
            "Properties": {
                "SandboxId": "RETAIL",
                "UserTokens": [xbl_token]
            },
            "RelyingParty": "rp://api.minecraftservices.com/",
            "TokenType": "JWT"
        });
        let response = self.client.post(XSTS_AUTH_URL).json(&body).send().await?;

        if response.status().is_success() {
            let parsed: XboxResponse = response.json().await?;
            return Ok(parsed.token);
        }

        let err: XstsError = response.json().await.unwrap_or(XstsError { xerr: 0 });
        let hint = match err.xerr {
            2148916233 => "this Microsoft account does not have an Xbox profile; create one first",
            2148916235 => "Xbox Live is not available in your region",
            2148916238 => "this account is a child account and must be added to a family",
            _ => "unknown XSTS error",
        };
        Err(CoreError::Auth(format!(
            "XSTS authorization failed ({}) : {hint}",
            err.xerr
        )))
    }

    async fn minecraft_login(&self, uhs: &str, xsts_token: &str) -> Result<(String, Option<u64>)> {
        let body = serde_json::json!({
            "identityToken": format!("XBL3.0 x={uhs};{xsts_token}")
        });
        let response = self.client.post(MC_LOGIN_URL).json(&body).send().await?;
        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CoreError::Auth(format!(
                "Minecraft services login failed: {text}"
            )));
        }
        let parsed: MinecraftAuthResponse = response.json().await?;
        Ok((parsed.access_token, parsed.expires_in))
    }

    /// Fetch the authenticated player's profile (id, name, skins).
    pub async fn fetch_profile(&self, mc_access_token: &str) -> Result<MinecraftProfile> {
        let response = self
            .client
            .get(MC_PROFILE_URL)
            .bearer_auth(mc_access_token)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::Auth(
                "this account does not own Minecraft: Java Edition".into(),
            ));
        }
        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CoreError::Auth(format!("profile lookup failed: {text}")));
        }
        Ok(response.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn active_skin_selection() {
        let profile = MinecraftProfile {
            id: "abc".into(),
            name: "Steve".into(),
            skins: vec![
                ProfileSkin {
                    id: "1".into(),
                    state: "INACTIVE".into(),
                    url: "http://inactive".into(),
                    variant: None,
                },
                ProfileSkin {
                    id: "2".into(),
                    state: "ACTIVE".into(),
                    url: "http://active".into(),
                    variant: Some("classic".into()),
                },
            ],
            capes: vec![],
        };
        assert_eq!(profile.active_skin(), Some("http://active"));
    }

    #[test]
    fn device_code_prompt_deserializes() {
        let raw = r#"{
            "user_code": "ABCD-EFGH",
            "device_code": "secret",
            "verification_uri": "https://microsoft.com/link",
            "expires_in": 900,
            "interval": 5,
            "message": "go to the link"
        }"#;
        let prompt: DeviceCodePrompt = serde_json::from_str(raw).unwrap();
        assert_eq!(prompt.user_code, "ABCD-EFGH");
        assert_eq!(prompt.expires_in(), Duration::from_secs(900));
    }
}
