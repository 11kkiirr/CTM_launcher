//! Mojang skin management and render URL helpers.

use serde::{Deserialize, Serialize};

use crate::auth::microsoft::MinecraftProfile;
use crate::error::{CoreError, Result};

const SKINS_URL: &str = "https://api.minecraftservices.com/minecraft/profile/skins";
const PROFILE_URL: &str = "https://api.minecraftservices.com/minecraft/profile";

/// The two supported skin models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkinVariant {
    Classic,
    Slim,
}

impl SkinVariant {
    pub fn as_str(&self) -> &'static str {
        match self {
            SkinVariant::Classic => "classic",
            SkinVariant::Slim => "slim",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SkinVariant::Classic => "Classic (Steve)",
            SkinVariant::Slim => "Slim (Alex)",
        }
    }
}

/// Client for the Mojang skin/profile endpoints.
#[derive(Debug, Clone)]
pub struct SkinClient {
    client: reqwest::Client,
}

impl SkinClient {
    pub fn new(client: reqwest::Client) -> Self {
        Self { client }
    }

    /// Fetch the authenticated profile including skins.
    pub async fn profile(&self, access_token: &str) -> Result<MinecraftProfile> {
        let response = self
            .client
            .get(PROFILE_URL)
            .bearer_auth(access_token)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(CoreError::Skin(format!(
                "profile lookup failed with {}",
                response.status()
            )));
        }
        Ok(response.json().await?)
    }

    /// Apply a skin from a public URL (Mojang fetches it).
    pub async fn set_skin_url(
        &self,
        access_token: &str,
        url: &str,
        variant: SkinVariant,
    ) -> Result<()> {
        let body = serde_json::json!({
            "variant": variant.as_str(),
            "url": url,
        });
        let response = self
            .client
            .post(SKINS_URL)
            .bearer_auth(access_token)
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CoreError::Skin(format!("failed to set skin: {text}")));
        }
        Ok(())
    }

    /// Upload a skin PNG file.
    pub async fn upload_skin(
        &self,
        access_token: &str,
        bytes: Vec<u8>,
        filename: &str,
        variant: SkinVariant,
    ) -> Result<()> {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename.to_string())
            .mime_str("image/png")
            .map_err(|e| CoreError::Skin(e.to_string()))?;
        let form = reqwest::multipart::Form::new()
            .text("variant", variant.as_str())
            .part("file", part);

        let response = self
            .client
            .post(SKINS_URL)
            .bearer_auth(access_token)
            .multipart(form)
            .send()
            .await?;
        if !response.status().is_success() {
            let text = response.text().await.unwrap_or_default();
            return Err(CoreError::Skin(format!("failed to upload skin: {text}")));
        }
        Ok(())
    }

    /// Remove the active skin, reverting to the default.
    pub async fn reset_skin(&self, access_token: &str) -> Result<()> {
        let response = self
            .client
            .delete(SKINS_URL)
            .bearer_auth(access_token)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(CoreError::Skin(format!(
                "failed to reset skin with {}",
                response.status()
            )));
        }
        Ok(())
    }
}

/// Crafatar head render URL (with overlay) for a UUID.
pub fn head_render_url(uuid: &str, size: u32) -> String {
    format!("https://crafatar.com/avatars/{uuid}?size={size}&overlay")
}

/// Full-body render URL for a UUID.
pub fn body_render_url(uuid: &str) -> String {
    format!("https://crafatar.com/renders/body/{uuid}?overlay")
}

/// Raw skin texture URL for a UUID.
pub fn skin_texture_url(uuid: &str) -> String {
    format!("https://crafatar.com/skins/{uuid}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_urls_contain_uuid() {
        let uuid = "b50ad385-829d-3141-a216-7e7d7539ba7f";
        assert!(head_render_url(uuid, 128).contains(uuid));
        assert!(body_render_url(uuid).contains(uuid));
        assert_eq!(SkinVariant::Slim.as_str(), "slim");
    }
}
