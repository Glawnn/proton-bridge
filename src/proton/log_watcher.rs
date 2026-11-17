use crate::proton::{PortMapping, PortMappingTracker, is_mapping_expired};
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use std::fs;
use std::io::{self, Read, Seek};
use std::path::PathBuf;
use std::time::SystemTime;

/// Surveille un fichier de logs Proton VPN de manière incrémentale.
///
/// Responsabilités :
/// - lire le fichier au démarrage pour récupérer le dernier état connu ;
/// - ne traiter que les nouvelles lignes par la suite ;
/// - détecter la rotation (`client-logs.txt` remplacé / `client-logs.1.txt`) ;
/// - gérer l'indisponibilité temporaire du fichier ;
/// - invalider le mapping quand Proton est déconnecté ou le mapping expiré.
pub struct LogWatcher {
    path: PathBuf,
    tracker: PortMappingTracker,
    offset: u64,
    last_size: u64,
    last_modified: Option<std::time::SystemTime>,
}

impl LogWatcher {
    pub fn new(path: PathBuf) -> Result<Self> {
        let metadata = fs::metadata(&path)
            .with_context(|| format!("Impossible d'accéder au fichier de logs {:?}", path))?;
        let size = metadata.len();
        let modified = metadata.modified().ok();

        let mut tracker = PortMappingTracker::new();

        // Lecture du fichier au démarrage pour récupérer le dernier état connu.
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Impossible de lire le fichier de logs {:?}", path))?;
        tracker.feed_lines(&content);

        Ok(Self {
            path,
            tracker,
            offset: size,
            last_size: size,
            last_modified: modified,
        })
    }

    /// Retourne le mapping valide courant, ou `None` s'il n'y en a pas
    /// (déconnecté, expiré, ou fichier indisponible).
    pub fn poll(&mut self) -> Result<Option<PortMapping>> {
        let metadata = match fs::metadata(&self.path) {
            Ok(m) => m,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // Fichier temporairement indisponible (Proton redémarre, rotation en cours).
                // On ne conserve pas l'ancien mapping : on ne sait pas si Proton est
                // toujours connecté.
                self.tracker.invalidate();
                self.offset = 0;
                self.last_size = 0;
                self.last_modified = None;
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };

        let size = metadata.len();
        let modified = metadata.modified().ok();

        // Détection de rotation / troncature / remplacement du fichier.
        let rotated = size < self.offset
            || (self.last_size > 0 && size == 0)
            || (modified.is_some()
                && self.last_modified.is_some()
                && modified != self.last_modified
                && size == self.last_size
                && self.last_size > 0);

        if rotated {
            self.tracker.invalidate();
            self.offset = 0;
            self.last_size = 0;
            self.last_modified = None;
        }

        // Lecture des nouvelles lignes uniquement.
        let new_content = if size <= self.offset {
            String::new()
        } else {
            read_from_offset(&self.path, self.offset).with_context(|| {
                format!("Impossible de lire les nouveaux logs de {:?}", self.path)
            })?
        };

        if !new_content.is_empty() {
            self.tracker.feed_lines(&new_content);
            self.offset = size;
        }

        self.last_size = size;
        self.last_modified = modified;

        let now: DateTime<Utc> = SystemTime::now().into();
        match self.tracker.mapping() {
            Some(m) if !is_mapping_expired(m, now) => Ok(Some(m.clone())),
            _ => {
                // Mapping expiré ou absent : on invalide pour ne pas réutiliser un
                // ancien port après une déconnexion.
                self.tracker.invalidate();
                Ok(None)
            }
        }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

fn read_from_offset(path: &PathBuf, offset: u64) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    file.seek(std::io::SeekFrom::Start(offset))?;
    let mut content = String::new();
    file.read_to_string(&mut content)?;
    Ok(content)
}
