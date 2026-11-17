use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;
const GITHUB_API_RELEASES_DEFAULT: &str =
    "https://api.github.com/repos/Glawnn/proton-bridge/releases/latest";

/// URL de l'API GitHub Releases (surchargeable via PROTON_BRIDGE_GITHUB_API_URL pour les tests).
fn github_api_url() -> String {
    std::env::var("PROTON_BRIDGE_GITHUB_API_URL")
        .unwrap_or_else(|_| GITHUB_API_RELEASES_DEFAULT.to_string())
}

/// Infos d'une release GitHub pertinente pour la mise à jour.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseInfo {
    pub version: String,
    pub download_url: String,
}

/// État échangé entre proton-bridge.exe et proton-bridge-updater.exe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateState {
    pub pending_update: bool,
    pub new_version: Option<String>,
    pub download_path: Option<String>,
}

impl Default for UpdateState {
    fn default() -> Self {
        Self {
            pending_update: false,
            new_version: None,
            download_path: None,
        }
    }
}

/// %APPDATA%\proton-bridge\update-state.json
pub fn update_state_path() -> Result<PathBuf> {
    env::var("APPDATA")
        .map(|p| {
            PathBuf::from(p)
                .join("proton-bridge")
                .join("update-state.json")
        })
        .context("La variable d'environnement APPDATA est introuvable (Windows requis)")
}

/// Répertoire de téléchargement temporaire des mises à jour : %TEMP%\proton-bridge-update\
pub fn update_download_dir() -> Result<PathBuf> {
    env::var("TEMP")
        .map(|t| PathBuf::from(t).join("proton-bridge-update"))
        .context("La variable d'environnement TEMP est introuvable (Windows requis)")
}

/// Client léger pour interroger l'API GitHub Releases.
pub struct UpdateClient {
    client: Client,
}

impl UpdateClient {
    pub fn new() -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("proton-bridge-updater")
            .build()?;
        Ok(Self { client })
    }

    /// Vérifie s'il existe une version plus récente sur GitHub.
    pub async fn check_for_update(&self, current_version: &str) -> Result<Option<ReleaseInfo>> {
        let url = github_api_url();
        let response = self
            .client
            .get(&url)
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .with_context(|| format!("Impossible de contacter l'API GitHub Releases ({url})"))?;

        if !response.status().is_success() {
            anyhow::bail!("API GitHub Releases a retourné HTTP {}", response.status());
        }

        let release: GitHubRelease = response
            .json()
            .await
            .context("Impossible de parser la réponse GitHub")?;

        let remote_version = release.tag_name.trim_start_matches('v');
        let current =
            semver::Version::parse(current_version).context("Version actuelle invalide")?;
        let remote = semver::Version::parse(remote_version).context("Version distante invalide")?;

        if remote <= current {
            return Ok(None);
        }

        // Chercher l'asset proton-bridge.exe
        let asset = release
            .assets
            .into_iter()
            .find(|a| a.name == "proton-bridge.exe")
            .context("Aucun asset proton-bridge.exe trouvé dans la release")?;

        Ok(Some(ReleaseInfo {
            version: remote.to_string(),
            download_url: asset.browser_download_url,
        }))
    }

    /// Télécharge un binaire depuis une URL vers un fichier de destination.
    pub async fn download(&self, url: &str, dest: &Path) -> Result<()> {
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("Impossible de créer {:?}", parent))?;
        }

        let response = self
            .client
            .get(url)
            .send()
            .await
            .context("Impossible de télécharger la mise à jour")?;

        if !response.status().is_success() {
            anyhow::bail!("Échec du téléchargement (HTTP {})", response.status());
        }

        let bytes = response
            .bytes()
            .await
            .context("Impossible de lire le corps de la réponse")?;
        std::fs::write(dest, &bytes).with_context(|| format!("Impossible d'écrire {:?}", dest))?;

        Ok(())
    }
}

/// Réponse simplifiée de l'API GitHub Releases.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Écrit l'état de mise à jour dans le fichier partagé.
pub fn write_update_state(state: &UpdateState) -> Result<()> {
    let path = update_state_path()?;
    let content = serde_json::to_string_pretty(state)
        .context("Impossible de sérialiser l'état de mise à jour")?;
    std::fs::write(&path, content).with_context(|| format!("Impossible d'écrire {:?}", path))?;
    Ok(())
}

/// Lit l'état de mise à jour depuis le fichier partagé.
pub fn read_update_state() -> Result<UpdateState> {
    let path = update_state_path()?;
    let content =
        std::fs::read_to_string(&path).with_context(|| format!("Impossible de lire {:?}", path))?;
    let state: UpdateState =
        serde_json::from_str(&content).context("Impossible de parser l'état de mise à jour")?;
    Ok(state)
}

/// Supprime le fichier d'état de mise à jour.
pub fn clear_update_state() -> Result<()> {
    let path = update_state_path()?;
    if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("Impossible de supprimer {:?}", path))?;
    }
    Ok(())
}
