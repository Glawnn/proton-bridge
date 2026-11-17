pub mod setup;

pub use setup::{SetupConfig, ensure_config};

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

const CONFIG_FILE_NAME: &str = "config.toml";
const LOGS_DIR_NAME: &str = "logs";
const APP_NAME: &str = "proton-bridge";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub qbittorrent: QbittorrentSettings,
    pub proton: ProtonSettings,
    pub general: GeneralSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QbittorrentSettings {
    pub url: String,
    pub username: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProtonSettings {
    pub log_path: PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub log_level: String,
    pub poll_interval_seconds: u64,
}

impl Config {
    /// %APPDATA%\proton-bridge\
    pub fn config_dir() -> anyhow::Result<PathBuf> {
        app_data_dir().map(|p| p.join(APP_NAME))
    }

    pub fn config_path() -> anyhow::Result<PathBuf> {
        Self::config_dir().map(|p| p.join(CONFIG_FILE_NAME))
    }

    pub fn logs_dir() -> anyhow::Result<PathBuf> {
        Self::config_dir().map(|p| p.join(LOGS_DIR_NAME))
    }

    pub fn log_file_path() -> anyhow::Result<PathBuf> {
        Self::logs_dir().map(|p| p.join("proton-bridge.log"))
    }

    pub fn load() -> anyhow::Result<Self> {
        let path = Self::config_path()?;
        let content = fs::read_to_string(&path).with_context(|| {
            format!("Impossible de lire le fichier de configuration {:?}", path)
        })?;
        let config: Config = toml::from_str(&content).with_context(|| {
            format!(
                "Impossible de parser le fichier de configuration {:?}",
                path
            )
        })?;
        Ok(config)
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let dir = Self::config_dir()?;
        fs::create_dir_all(&dir)?;
        let path = Self::config_path()?;
        let content =
            toml::to_string_pretty(self).context("Impossible de sérialiser la configuration")?;
        fs::write(&path, content).with_context(|| {
            format!("Impossible d'écrire le fichier de configuration {:?}", path)
        })?;
        Ok(())
    }

    pub fn exists() -> bool {
        Self::config_path().map(|p| p.exists()).unwrap_or(false)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            qbittorrent: QbittorrentSettings {
                url: String::new(),
                username: String::new(),
                password: String::new(),
            },
            proton: ProtonSettings {
                log_path: PathBuf::new(),
            },
            general: GeneralSettings {
                log_level: "info".to_string(),
                poll_interval_seconds: 3,
            },
        }
    }
}

/// %APPDATA% (données utilisateur Windows).
fn app_data_dir() -> anyhow::Result<PathBuf> {
    std::env::var("APPDATA")
        .map(PathBuf::from)
        .context("La variable d'environnement APPDATA est introuvable (Windows requis)")
}

/// Détecte le chemin par défaut des logs Proton VPN sous Windows.
pub fn default_proton_log_path() -> Option<PathBuf> {
    let local = std::env::var("LOCALAPPDATA").ok()?;
    let mut path = PathBuf::from(local);
    path.push("Proton");
    path.push("Proton VPN");
    path.push("Logs");
    path.push("client-logs.txt");
    if path.exists() { Some(path) } else { None }
}
