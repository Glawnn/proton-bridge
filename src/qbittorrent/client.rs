use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde_json::{Value, json};
use std::time::Duration;

pub struct QBittorrentClient {
    client: Client,
    base_url: String,
    username: String,
    password: String,
}

const MAX_LOGIN_RETRIES: u32 = 3;
const MAX_PORT_SET_RETRIES: u32 = 3;

impl QBittorrentClient {
    pub async fn new(base_url: impl Into<String>, username: &str, password: &str) -> Result<Self> {
        let client = Client::builder()
            .cookie_store(true)
            .timeout(Duration::from_secs(10))
            .build()?;

        let base_url = base_url.into();
        let username = username.to_string();
        let password = password.to_string();

        let me = Self {
            client: client.clone(),
            base_url: base_url.clone(),
            username: username.clone(),
            password: password.clone(),
        };

        me.login_with_retries().await?;

        Ok(Self {
            client,
            base_url,
            username,
            password,
        })
    }

    async fn login_with_retries(&self) -> Result<()> {
        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 1..=MAX_LOGIN_RETRIES {
            match self.attempt_login().await {
                Ok(()) => return Ok(()),
                Err(e) => {
                    last_error = Some(e);
                    if attempt < MAX_LOGIN_RETRIES {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
        }
        bail!(last_error.unwrap_or_else(|| anyhow::anyhow!("Connexion à qBittorrent impossible")))
    }

    async fn attempt_login(&self) -> Result<()> {
        let response = self
            .client
            .post(format!("{}/api/v2/auth/login", self.base_url))
            .form(&[("username", &self.username), ("password", &self.password)])
            .send()
            .await
            .context(
                "Impossible de se connecter à qBittorrent (vérifiez que l'application est lancée)",
            )?;

        let status = response.status();
        let body = response.text().await?;

        if !status.is_success() {
            bail!("Authentification qBittorrent échouée (HTTP {status}): {body:?}");
        }

        if status.as_u16() != 204 && body.trim() != "Ok." {
            bail!("Réponse inattendue de l'authentification qBittorrent: {body:?}");
        }

        Ok(())
    }

    /// Récupère le port d'écoute actuellement configuré dans qBittorrent.
    pub async fn get_listen_port(&self) -> Result<u16> {
        let response = self
            .client
            .get(format!("{}/api/v2/app/preferences", self.base_url))
            .send()
            .await?
            .error_for_status()?;

        let preferences: Value = response.json().await?;

        let port = preferences["listen_port"]
            .as_u64()
            .context("Préférence listen_port introuvable dans qBittorrent")?;

        Ok(u16::try_from(port)?)
    }

    /// Modifie le port d'écoute de qBittorrent et vérifie que la modification
    /// a bien été prise en compte. Réessaie en cas d'échec temporaire.
    pub async fn set_listen_port(&self, port: u16) -> Result<()> {
        if port == 0 {
            bail!("Le port ne peut pas être zéro");
        }

        let mut last_error: Option<anyhow::Error> = None;
        for attempt in 1..=MAX_PORT_SET_RETRIES {
            match self.attempt_set_listen_port(port).await {
                Ok(()) => {
                    // Vérification que le port a bien été appliqué.
                    let actual = self.get_listen_port().await?;
                    if actual == port {
                        return Ok(());
                    }
                    last_error = Some(anyhow::anyhow!(
                        "Le port qBittorrent n'a pas été appliqué (demandé {port}, obtenu {actual})"
                    ));
                }
                Err(e) => {
                    last_error = Some(e);
                }
            }
            if attempt < MAX_PORT_SET_RETRIES {
                // Nouvelle authentification puis nouvelle tentative.
                self.login_with_retries().await?;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }

        bail!(
            last_error
                .unwrap_or_else(|| anyhow::anyhow!("Impossible de modifier le port qBittorrent"))
        );
    }

    async fn attempt_set_listen_port(&self, port: u16) -> Result<()> {
        let preferences = json!({
            "listen_port": port
        });

        let response = self
            .client
            .post(format!("{}/api/v2/app/setPreferences", self.base_url))
            .form(&[("json", preferences.to_string())])
            .send()
            .await?;

        let status = response.status();

        if !status.is_success() {
            bail!("Échec de la modification du port qBittorrent: HTTP {status}");
        }

        Ok(())
    }
}
