use crate::proton::PortMapping;
use crate::qbittorrent::QBittorrentClient;
use anyhow::Result;
use tracing::{debug, info, warn};

/// Résultat d'une tentative de synchronisation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncOutcome {
    /// Aucun changement nécessaire (port déjà correct ou pas de mapping valide).
    NoChange,
    /// Le port a été modifié avec succès.
    Updated { previous: u16, new: u16 },
}

/// Effectue la synchronisation entre le mapping Proton et le port qBittorrent.
///
/// Règles :
/// - On ne modifie qBittorrent que si un mapping valide est disponible et que son
///   port diffère de celui actuellement configuré.
/// - En cas d'erreur qBittorrent, on retourne l'erreur sans bloquer le programme
///   (l'appelant décidera de réessayer au prochain cycle).
/// - On ne considère jamais un ancien mapping comme valide : l'expiration est déjà
///   gérée par le watcher (le mapping passé ici est déjà filtré).
pub async fn sync(
    qbittorrent: &QBittorrentClient,
    mapping: Option<&PortMapping>,
    current_port: &mut u16,
) -> Result<SyncOutcome> {
    let target_port = match mapping {
        Some(m) => m.port,
        None => {
            debug!("Aucun mapping Proton valide : aucune modification de qBittorrent.");
            return Ok(SyncOutcome::NoChange);
        }
    };

    if target_port == *current_port {
        debug!("Le port qBittorrent ({target_port}) est déjà celui du mapping Proton.");
        return Ok(SyncOutcome::NoChange);
    }

    info!(
        "Changement de port Proton détecté : {} -> {}",
        *current_port, target_port
    );

    qbittorrent.set_listen_port(target_port).await?;

    let previous = *current_port;
    *current_port = target_port;
    info!("Port qBittorrent mis à jour avec succès : {target_port}.");

    Ok(SyncOutcome::Updated {
        previous,
        new: target_port,
    })
}
