use crate::config::{Config, GeneralSettings, ProtonSettings, QbittorrentSettings};
use anyhow::{Context, Result};
use std::io::{self, Write};
use std::path::PathBuf;

pub struct SetupConfig {
    pub qbittorrent_url: String,
    pub qbittorrent_username: String,
    pub qbittorrent_password: String,
    pub proton_log_path: String,
    pub log_level: String,
    pub poll_interval_seconds: u64,
}

fn read_line(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn read_password(prompt: &str) -> Result<String> {
    // Lecture simple (pas de masquage sans dépendance externe).
    read_line(prompt)
}

fn ask_required(prompt: &str) -> Result<String> {
    loop {
        let value = read_line(prompt)?;
        if !value.is_empty() {
            return Ok(value);
        }
        println!("Ce champ est obligatoire. Veuillez saisir une valeur.");
    }
}

fn ask_with_default(prompt: &str, default: &str) -> Result<String> {
    let value = read_line(&format!("{prompt} [{default}] "))?;
    if value.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value)
    }
}

pub async fn ensure_config() -> Result<Config> {
    if Config::exists() {
        println!("Configuration existante trouvée, chargement...");
        return Config::load();
    }

    println!("=== Première configuration de Proton Bridge ===");
    println!("Aucune configuration trouvée. Nous allons configurer le logiciel.\n");

    // 1. Détection automatique du chemin des logs Proton.
    let detected = crate::config::default_proton_log_path();
    let proton_log_path = match &detected {
        Some(path) => {
            println!(
                "Chemin des logs Proton détecté automatiquement : {}",
                path.display()
            );
            let answer = read_line("Voulez-vous utiliser ce chemin ? (o/N) ")?;
            if answer.eq_ignore_ascii_case("o") || answer.eq_ignore_ascii_case("oui") {
                path.to_string_lossy().to_string()
            } else {
                ask_required("Chemin vers le fichier client-logs.txt de Proton VPN : ")?
            }
        }
        None => {
            println!("Chemin des logs Proton non détecté automatiquement.");
            ask_required("Chemin vers le fichier client-logs.txt de Proton VPN : ")?
        }
    };

    // 2. Informations de connexion qBittorrent.
    let qbittorrent_url = ask_with_default("URL de qBittorrent (Web UI)", "http://localhost:8080")?;
    let qbittorrent_username = ask_required("Nom d'utilisateur qBittorrent : ")?;
    let qbittorrent_password = read_password("Mot de passe qBittorrent : ")?;

    // 3. Réglages généraux.
    let log_level = ask_with_default("Niveau de logs (debug/info/warn/error)", "info")?;
    let poll_interval: u64 = loop {
        let raw = ask_with_default("Intervalle de vérification (secondes)", "3")?;
        match raw.parse::<u64>() {
            Ok(v) if v > 0 => break v,
            _ => println!("Veuillez saisir un nombre entier supérieur à 0."),
        }
    };

    let setup = SetupConfig {
        qbittorrent_url,
        qbittorrent_username,
        qbittorrent_password,
        proton_log_path,
        log_level,
        poll_interval_seconds: poll_interval,
    };

    let config = Config {
        qbittorrent: QbittorrentSettings {
            url: setup.qbittorrent_url,
            username: setup.qbittorrent_username,
            password: setup.qbittorrent_password,
        },
        proton: ProtonSettings {
            log_path: PathBuf::from(setup.proton_log_path),
        },
        general: GeneralSettings {
            log_level: setup.log_level,
            poll_interval_seconds: setup.poll_interval_seconds,
        },
    };

    // 4. Test de connexion à qBittorrent avant d'enregistrer.
    println!("\nTest de la connexion à qBittorrent...");
    let client = crate::qbittorrent::QBittorrentClient::new(
        &config.qbittorrent.url,
        &config.qbittorrent.username,
        &config.qbittorrent.password,
    )
    .await
    .context("La connexion à qBittorrent a échoué. Vérifiez l'URL, le nom d'utilisateur et le mot de passe.")?;
    let _port = client
        .get_listen_port()
        .await
        .context("Impossible de lire les préférences de qBittorrent")?;
    println!("Connexion à qBittorrent réussie (port actuel : {_port}).");

    // 5. Enregistrement de la configuration.
    config
        .save()
        .context("Impossible d'enregistrer la configuration")?;
    println!(
        "Configuration enregistrée dans {:?}",
        Config::config_path()?
    );

    Ok(config)
}
