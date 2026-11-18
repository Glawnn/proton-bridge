mod config;
mod proton;
mod qbittorrent;
mod sync;
mod update;

use anyhow::{Context, Result};
use config::Config;
use proton::LogWatcher;
use qbittorrent::QBittorrentClient;
use std::env;
use std::process::Command;
use sync::sync;
use tokio::time::{Duration, sleep};
use tracing::{debug, info, warn};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialisation des logs : console + fichier (avec rotation), niveau info par défaut.
    init_tracing()?;

    info!("Démarrage de Proton Bridge");

    // Chargement ou création de la configuration.
    let config = if Config::exists() {
        info!("Chargement de la configuration existante.");
        Config::load().context("Impossible de charger la configuration")?
    } else {
        info!("Aucune configuration trouvée, lancement de l'assistant de première configuration.");
        config::ensure_config().await?
    };

    info!(
        "Configuration chargée : qBittorrent={} utilisateur={} logs_proton={}",
        config.qbittorrent.url,
        config.qbittorrent.username,
        config.proton.log_path.display()
    );

    // Vérification automatique des mises à jour au démarrage.
    if config.general.auto_update {
        if let Err(e) = check_and_apply_update().await {
            warn!("Échec de la vérification de mise à jour : {e:#}");
        }
    }

    // Connexion à qBittorrent.
    let qbittorrent = QBittorrentClient::new(
        &config.qbittorrent.url,
        &config.qbittorrent.username,
        &config.qbittorrent.password,
    )
    .await
    .context("Impossible de se connecter à qBittorrent")?;

    let mut current_port = qbittorrent.get_listen_port().await?;
    info!("Connecté à qBittorrent. Port d'écoute actuel : {current_port}");

    // Surveillance des logs Proton.
    let mut log_watcher = LogWatcher::new(config.proton.log_path.clone())
        .context("Impossible d'initialiser la surveillance des logs Proton")?;
    info!(
        "Surveillance des logs Proton : {}",
        config.proton.log_path.display()
    );

    let poll_interval = Duration::from_secs(config.general.poll_interval_seconds);

    loop {
        match log_watcher.poll() {
            Ok(Some(mapping)) => {
                if let Err(e) = sync(&qbittorrent, Some(&mapping), &mut current_port).await {
                    warn!(
                        "Échec de la synchronisation : {e:#}. Nouvelle tentative au prochain cycle."
                    );
                }
            }
            Ok(None) => {
                // Pas de mapping valide : rien à faire, on ne modifie pas qBittorrent.
                debug!("Aucun mapping Proton valide disponible.");
            }
            Err(e) => {
                warn!(
                    "Erreur lors de la lecture des logs Proton : {e:#}. Nouvelle tentative au prochain cycle."
                );
            }
        }

        sleep(poll_interval).await;
    }
}

/// Vérifie s'il existe une mise à jour et l'applique si nécessaire.
///
/// Lance proton-bridge-updater.exe en mode --check, puis --do-update si une
/// mise à jour est en attente. Si --do-update est exécuté, le nouveau
/// executable est lancé et la présente instance se termine.
async fn check_and_apply_update() -> Result<()> {
    let exe_path = env::current_exe()?;
    let updater_path = exe_path.with_file_name("proton-bridge-updater.exe");

    if !updater_path.exists() {
        info!(
            "Updater introuvable ({:?}), mise à jour ignorée.",
            updater_path
        );
        return Ok(());
    }

    let current_version = env!("CARGO_PKG_VERSION");

    info!("Vérification des mises à jour (version {current_version})...");

    let _output = Command::new(&updater_path)
        .args([
            "--check",
            "--current-version",
            current_version,
            "--main-exe",
            &exe_path.to_string_lossy(),
        ])
        .spawn()?
        .wait_with_output()
        .context("Impossible d'exécuter proton-bridge-updater --check")?;

    // Lecture de l'état partagé pour savoir si une mise à jour est en attente.
    match update::read_update_state() {
        Ok(state) if state.pending_update => {
            info!("Mise à jour en attente, application via l'updater...");
            let _output = Command::new(&updater_path)
                .args(["--do-update", "--main-exe", &exe_path.to_string_lossy()])
                .spawn()?
                .wait_with_output()
                .context("Impossible d'exécuter proton-bridge-updater --do-update")?;

            // Le nouvel executable a été lancé par l'updater, on se termine.
            return Ok(());
        }
        Ok(_) => {
            info!("Aucune mise à jour requise.");
        }
        Err(e) => {
            warn!("Impossible de lire l'état de mise à jour : {e:#}");
        }
    }

    Ok(())
}

fn init_tracing() -> Result<()> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));

    let console_layer = tracing_subscriber::fmt::layer();

    // Répertoire des logs : %APPDATA%\proton-bridge\logs\
    let logs_dir = Config::logs_dir()?;
    std::fs::create_dir_all(&logs_dir)?;
    // Rotation quotidienne des fichiers de logs pour éviter une croissance infinie.
    let file_appender = tracing_appender::rolling::daily(logs_dir, "proton-bridge.log");
    let file_layer = tracing_subscriber::fmt::layer()
        .with_writer(file_appender)
        .with_ansi(false);

    tracing_subscriber::registry()
        .with(filter)
        .with(console_layer)
        .with(file_layer)
        .init();

    Ok(())
}
