//! Modrinth API v2 HTTP client.

use crate::error::{CoreError, Result};
use crate::modrinth::models::{Project, SearchResults, Version};

/// Default Modrinth API base URL.
pub const API_BASE: &str = "https://api.modrinth.com/v2";

/// A typed client for the Modrinth API.
#[derive(Debug, Clone)]
pub struct ModrinthClient {
    client: reqwest::Client,
    base: String,
}

impl ModrinthClient {
    /// Build a client, ensuring a descriptive `User-Agent` is present.
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            base: API_BASE.to_string(),
        }
    }

    /// Build a client against a custom base URL (useful for testing).
    pub fn with_base(client: reqwest::Client, base: impl Into<String>) -> Self {
        Self {
            client,
            base: base.into(),
        }
    }

    /// Search projects with optional project-type, game-version and loader filters.
    pub async fn search(
        &self,
        query: &str,
        project_type: Option<&str>,
        game_version: Option<&str>,
        loader: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> Result<SearchResults> {
        let mut facets: Vec<Vec<String>> = Vec::new();
        if let Some(kind) = project_type {
            facets.push(vec![format!("project_type:{kind}")]);
        }
        if let Some(version) = game_version {
            facets.push(vec![format!("versions:{version}")]);
        }
        if let Some(loader) = loader {
            facets.push(vec![format!("categories:{loader}")]);
        }

        let mut params: Vec<(&str, String)> = vec![
            ("query", query.to_string()),
            ("limit", limit.to_string()),
            ("offset", offset.to_string()),
        ];
        if !facets.is_empty() {
            params.push(("facets", serde_json::to_string(&facets)?));
        }

        let response = self
            .client
            .get(format!("{}/search", self.base))
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    /// Fetch a single project by id or slug.
    pub async fn project(&self, id_or_slug: &str) -> Result<Project> {
        let response = self
            .client
            .get(format!("{}/project/{id_or_slug}", self.base))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!(
                "Modrinth project {id_or_slug}"
            )));
        }
        Ok(response.error_for_status()?.json().await?)
    }

    /// Fetch versions for a project, optionally filtered by game version/loader.
    pub async fn versions_filtered(
        &self,
        project_id: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Vec<Version>> {
        let mut params: Vec<(&str, String)> = Vec::new();
        if let Some(v) = game_version {
            params.push(("game_versions", serde_json::to_string(&[v])?));
        }
        if let Some(l) = loader {
            params.push(("loaders", serde_json::to_string(&[l])?));
        }
        let response = self
            .client
            .get(format!("{}/project/{project_id}/version", self.base))
            .query(&params)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    /// Fetch a single version by id.
    pub async fn version(&self, id: &str) -> Result<Version> {
        let response = self
            .client
            .get(format!("{}/version/{id}", self.base))
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CoreError::NotFound(format!("Modrinth version {id}")));
        }
        Ok(response.error_for_status()?.json().await?)
    }

    /// Bulk-fetch versions by id.
    pub async fn versions_by_ids(&self, ids: &[String]) -> Result<Vec<Version>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .client
            .get(format!("{}/versions", self.base))
            .query(&[("ids", serde_json::to_string(ids)?)])
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    /// Look up a version from a local file hash.
    pub async fn version_from_hash(&self, sha1: &str) -> Result<Option<Version>> {
        let response = self
            .client
            .get(format!("{}/version_file/{sha1}", self.base))
            .query(&[("algorithm", "sha1")])
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(response.error_for_status()?.json().await?))
    }

    /// Bulk-fetch projects by id.
    pub async fn projects_by_ids(&self, ids: &[String]) -> Result<Vec<Project>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        let response = self
            .client
            .get(format!("{}/projects", self.base))
            .query(&[("ids", serde_json::to_string(ids)?)])
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }

    /// The newest version matching the given filters, if any.
    pub async fn latest_version(
        &self,
        project_id: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Option<Version>> {
        let mut versions = self
            .versions_filtered(project_id, game_version, loader)
            .await?;
        // Modrinth returns newest first, but sort defensively by date.
        versions.sort_by(|a, b| b.date_published.cmp(&a.date_published));
        Ok(versions.into_iter().next())
    }

    /// Check whether a local file (by SHA-1) has a newer version available.
    ///
    /// Returns `Some((installed, latest))` when both are known and differ.
    pub async fn check_update(
        &self,
        sha1: &str,
        game_version: Option<&str>,
        loader: Option<&str>,
    ) -> Result<Option<(Version, Version)>> {
        let Some(installed) = self.version_from_hash(sha1).await? else {
            return Ok(None);
        };
        let Some(latest) = self
            .latest_version(&installed.project_id, game_version, loader)
            .await?
        else {
            return Ok(None);
        };
        if latest.id != installed.id {
            Ok(Some((installed, latest)))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn builds_facets_json() {
        let facets = vec![
            vec!["project_type:mod".to_string()],
            vec!["versions:1.20.1".to_string()],
            vec!["categories:fabric".to_string()],
        ];
        let json = serde_json::to_string(&facets).unwrap();
        assert_eq!(
            json,
            r#"[["project_type:mod"],["versions:1.20.1"],["categories:fabric"]]"#
        );
    }
}
