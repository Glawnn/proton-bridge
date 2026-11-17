use chrono::{DateTime, NaiveDateTime, Utc};
use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMapping {
    pub port: u16,
    pub expires_at: DateTime<Utc>,
}

impl PortMapping {
    /// Retourne true si le mapping est encore valide (non expiré) à l'instant `now`.
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

const STATUS_REGEX_STR: &str = r"Received PortForwarding Status '([^']+)'";
const MAPPING_REGEX_STR: &str =
    r"Port pair (\d+)->(\d+), expiring in .*? at (\d{2}/\d{2}/\d{4} \d{2}:\d{2}:\d{2})";

fn status_regex() -> Regex {
    Regex::new(STATUS_REGEX_STR).expect("regex de statut invalide")
}

fn mapping_regex() -> Regex {
    Regex::new(MAPPING_REGEX_STR).expect("regex de mapping invalide")
}

/// Traite une seule ligne de log et met à jour le mapping en cours.
///
/// Cette fonction est utilisée à la fois par `parse_port_mapping` (tout le
/// contenu) et par `PortMappingTracker` (lecture incrémentale ligne par ligne).
pub fn apply_line(line: &str, mapping: &mut Option<PortMapping>) {
    let Some(status_captures) = status_regex().captures(line) else {
        return;
    };

    match &status_captures[1] {
        "SleepingUntilRefresh" => {
            let Some(captures) = mapping_regex().captures(line) else {
                *mapping = None;
                return;
            };

            let Ok(port) = captures[2].parse::<u16>() else {
                *mapping = None;
                return;
            };

            let Ok(expiration) = NaiveDateTime::parse_from_str(&captures[3], "%d/%m/%Y %H:%M:%S")
            else {
                *mapping = None;
                return;
            };

            if port == 0 {
                *mapping = None;
                return;
            }

            *mapping = Some(PortMapping {
                port,
                expires_at: expiration.and_utc(),
            });
        }
        "Stopped" | "Error" | "DestroyPortMappingCommunication" => {
            *mapping = None;
        }
        "Starting" | "HelloCommunication" | "PortMappingCommunication" => {
            // On conserve le mapping précédent pendant le renouvellement.
        }
        _ => {
            // Statut inconnu : on ne prend aucun risque avec un ancien port.
            *mapping = None;
        }
    }
}

/// Analyse l'intégralité d'un texte de logs et retourne le dernier mapping connu.
pub fn parse_port_mapping(logs: &str) -> Option<PortMapping> {
    let mut mapping: Option<PortMapping> = None;
    for line in logs.lines() {
        apply_line(line, &mut mapping);
    }
    mapping
}

/// Suiveur d'état de mapping conçu pour une lecture incrémentale des logs.
///
/// Il conserve le dernier mapping connu entre plusieurs appels à `feed_line`,
/// ce qui permet de ne traiter que les nouvelles lignes d'un fichier de log.
#[derive(Debug, Clone, Default)]
pub struct PortMappingTracker {
    mapping: Option<PortMapping>,
}

impl PortMappingTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed_line(&mut self, line: &str) {
        apply_line(line, &mut self.mapping);
    }

    pub fn feed_lines(&mut self, logs: &str) {
        for line in logs.lines() {
            self.feed_line(line);
        }
    }

    pub fn mapping(&self) -> Option<&PortMapping> {
        self.mapping.as_ref()
    }

    pub fn take_mapping(&mut self) -> Option<PortMapping> {
        self.mapping.take()
    }

    pub fn invalidate(&mut self) {
        self.mapping = None;
    }
}

pub fn is_mapping_expired(mapping: &PortMapping, now: DateTime<Utc>) -> bool {
    mapping.is_expired(now)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACTIVE: &str = "2026-11-17T19:53:58.706Z | INFO | PROCESS.COMM | Received PortForwarding Status 'SleepingUntilRefresh' triggered at '17/11/2026 19:53:58', Port pair 63536->63536, expiring in 00:01:00 at 17/11/2026 19:54:58 | {}";

    #[test]
    fn parses_valid_mapping() {
        let mapping = parse_port_mapping(ACTIVE).unwrap();
        assert_eq!(mapping.port, 63536);
    }

    #[test]
    fn stopped_invalidates_mapping() {
        let logs = format!(
            "{ACTIVE}\nReceived PortForwarding Status 'Stopped' triggered at '17/11/2026 19:55:00'"
        );
        assert_eq!(parse_port_mapping(&logs), None);
    }

    #[test]
    fn error_invalidates_mapping() {
        let logs = format!(
            "{ACTIVE}\nReceived PortForwarding Status 'Error' triggered at '17/11/2026 19:55:00'"
        );
        assert_eq!(parse_port_mapping(&logs), None);
    }

    #[test]
    fn renewal_keeps_mapping() {
        let logs = format!(
            "{ACTIVE}\nReceived PortForwarding Status 'PortMappingCommunication' triggered at '17/11/2026 19:54:28'"
        );
        assert_eq!(parse_port_mapping(&logs).unwrap().port, 63536);
    }

    #[test]
    fn new_mapping_after_stop_is_detected() {
        let logs = format!(
            "Received PortForwarding Status 'Stopped' triggered at '17/11/2026 19:32:06'\n{ACTIVE}"
        );
        assert_eq!(parse_port_mapping(&logs).unwrap().port, 63536);
    }

    #[test]
    fn tracker_incremental_parsing() {
        let mut tracker = PortMappingTracker::new();
        tracker.feed_line(ACTIVE);
        assert_eq!(tracker.mapping().unwrap().port, 63536);

        tracker.feed_line(
            "Received PortForwarding Status 'Stopped' triggered at '17/11/2026 19:55:00'",
        );
        assert!(tracker.mapping().is_none());

        tracker.feed_line(ACTIVE);
        assert_eq!(tracker.mapping().unwrap().port, 63536);
    }

    #[test]
    fn mapping_expiration() {
        let now = Utc::now();
        let expired = PortMapping {
            port: 12345,
            expires_at: now - chrono::Duration::seconds(1),
        };
        assert!(expired.is_expired(now));

        let valid = PortMapping {
            port: 12345,
            expires_at: now + chrono::Duration::seconds(60),
        };
        assert!(!valid.is_expired(now));
    }
}
