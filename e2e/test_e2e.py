"""Tests E2E de Proton Bridge.

Ces tests lancent le binaire Rust compilé en Release avec un APPDATA temporaire
et un mock qBittorrent. Ils vérifient que le binaire synchronise correctement le
port qBittorrent à partir des logs Proton simulés.

Scénario couverts :
- mapping actif -> port qBittorrent mis à jour
- statut Stopped -> aucun changement
- changement de port dans les logs -> nouveau port appliqué
- mapping expiré -> ignoré (aucun changement)
- qBittorrent indisponible au démarrage -> échec propre du binaire
- ports externe != interne -> seul le port interne est utilisé
- rotation des logs Proton -> détection, pas de réutilisation d'ancien port
- redémarrage Proton en cours -> invalidation puis nouveau mapping
- perte de session qBittorrent -> re-login + retry
- erreur temporaire qBittorrent -> retry non bloquant
- logs avec bruit -> parsing robuste
- port déjà correct -> aucun appel setPreferences inutile (idempotence)
"""

from __future__ import annotations

import os
import subprocess
import time
from pathlib import Path

from conftest import (
    LOG_LINE_ACTIVE_63537,
    LOG_LINE_ACTIVE_63538,
    LOG_LINE_EXPIRED,
    LOG_LINE_STOPPED,
    launch_bridge,
    write_config,
    write_proton_logs,
)

WAIT_SECONDS = 8
POLL_INTERVAL = 0.5


def _wait_for_set_port(qb_mock, expected_count: int, timeout: float = WAIT_SECONDS) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if qb_mock.set_port_count() >= expected_count:
            return True
        time.sleep(POLL_INTERVAL)
    return False


# ---------------------------------------------------------------------------
# Cas 1 : mapping actif -> qBittorrent est mis à jour
# ---------------------------------------------------------------------------

def test_sync_updates_qbittorrent_port(appdata, qb_mock, binary_path):
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a appelé aucun setPreferences"
        )

    assert qb_mock.last_set_port() == 63537, (
        f"Port attendu 63537, obtenu {qb_mock.last_set_port()}"
    )


# ---------------------------------------------------------------------------
# Cas 2 : statut Stopped -> aucun changement de port
# ---------------------------------------------------------------------------

def test_stopped_status_leaves_port_untouched(appdata, qb_mock, binary_path):
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_STOPPED)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        time.sleep(WAIT_SECONDS)

    assert qb_mock.set_port_count() == 0, (
        "Aucun setPreferences ne devrait être appelé après un statut Stopped"
    )


# ---------------------------------------------------------------------------
# Cas 3 : changement de port (dernier mapping dans le fichier l'emporte)
# ---------------------------------------------------------------------------

def test_port_change_is_applied(appdata, qb_mock, binary_path):
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537 + "\n" + LOG_LINE_ACTIVE_63538)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a appelé aucun setPreferences"
        )

    assert qb_mock.last_set_port() == 63538, (
        f"Port attendu 63538 (dernier mapping), obtenu {qb_mock.last_set_port()}"
    )


# ---------------------------------------------------------------------------
# Cas 4 : mapping expiré -> ignoré
# ---------------------------------------------------------------------------

def test_expired_mapping_is_ignored(appdata, qb_mock, binary_path):
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_EXPIRED)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        time.sleep(WAIT_SECONDS)

    assert qb_mock.set_port_count() == 0, (
        "Un mapping expiré ne doit pas déclencher de setPreferences"
    )


# ---------------------------------------------------------------------------
# Cas 5 : qBittorrent indisponible -> échec propre
# ---------------------------------------------------------------------------

def test_qbittorrent_unavailable_fails_gracefully(appdata, binary_path):
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537)
    # Port fermé : aucun mock n'écoute.
    write_config(
        appdata,
        url="http://127.0.0.1:19999",
        log_path=log_path,
    )

    env = os.environ.copy()
    env["APPDATA"] = appdata
    proc = subprocess.Popen([binary_path], env=env)
    try:
        return_code = proc.wait(timeout=10)
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()

    assert return_code != 0, (
        "Le binaire doit quitter avec une erreur si qBittorrent est indisponible"
    )


# ---------------------------------------------------------------------------
# Cas 6 : ports externe != interne -> seul le port interne est utilisé
# ---------------------------------------------------------------------------

def test_external_and_internal_port_are_different(appdata, qb_mock, binary_path):
    """Proton attribue un port externe (50000) et un port interne (63537).
    Le bridge doit utiliser le port interne (second) pour qBittorrent.
    """
    log_content = (
        "2026-11-17T19:53:58.706Z | INFO | PROCESS.COMM | "
        "Received PortForwarding Status 'SleepingUntilRefresh' triggered at '17/11/2026 19:53:58', "
        "Port pair 50000->63537, expiring in 00:01:00 at 17/11/2026 19:54:58 | {}"
    )
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, log_content)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a appelé aucun setPreferences"
        )

    assert qb_mock.last_set_port() == 63537, (
        f"Port interne attendu 63537, obtenu {qb_mock.last_set_port()}"
    )


# ---------------------------------------------------------------------------
# Cas 7 : rotation des logs Proton -> détection, pas de réutilisation
# ---------------------------------------------------------------------------

def test_log_rotation_detected(appdata, qb_mock, binary_path):
    """Le fichier client-logs.txt est remplacé (rotation Proton).
    Le bridge doit détecter la rotation et invalider l'ancien mapping.
    """
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata) as proc:
        # Attendre que le premier mapping soit appliqué.
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a appelé aucun setPreferences au premier cycle"
        )
        assert qb_mock.last_set_port() == 63537

        # Rotation : on remplace le fichier par un contenu sans mapping valide
        # (simule client-logs.1.txt qui remplace client-logs.txt).
        write_proton_logs(
            log_path,
            "2026-11-17T19:56:00.000Z | INFO | PROCESS.COMM | "
            "Autre ligne sans port forwarding | {}",
        )

        # Attendre quelques cycles : le bridge doit invalider l'ancien mapping
        # et ne PAS appeler de nouveau setPreferences avec un port différent.
        count_before = qb_mock.set_port_count()
        time.sleep(WAIT_SECONDS)

    assert qb_mock.set_port_count() == count_before, (
        "Après rotation sans nouveau mapping, aucun nouveau setPreferences ne doit être appelé"
    )


# ---------------------------------------------------------------------------
# Cas 8 : redémarrage Proton en cours -> invalidation puis nouveau mapping
# ---------------------------------------------------------------------------

def test_proton_restart_during_runtime(appdata, qb_mock, binary_path):
    """Le fichier de logs disparaît (Proton redémarre), puis réapparaît
    avec un nouveau mapping. Le bridge ne doit pas garder l'ancien port.
    """
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata) as proc:
        # Premier mapping appliqué.
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a appelé aucun setPreferences au premier cycle"
        )
        assert qb_mock.last_set_port() == 63537

        # Le fichier disparaît (Proton redémarre).
        Path(log_path).unlink(missing_ok=True)
        time.sleep(2)

        # Le fichier réapparaît avec un nouveau mapping.
        write_proton_logs(log_path, LOG_LINE_ACTIVE_63538)

        # Le bridge doit détecter le nouveau mapping et l'appliquer.
        assert _wait_for_set_port(qb_mock, 2), (
            "Le binaire n'a pas appliqué le nouveau mapping après redémarrage Proton"
        )

    assert qb_mock.last_set_port() == 63538, (
        f"Port attendu 63538 après redémarrage, obtenu {qb_mock.last_set_port()}"
    )


# ---------------------------------------------------------------------------
# Cas 9 : perte de session qBittorrent -> re-login + retry
# ---------------------------------------------------------------------------

def test_qbittorrent_session_loss_recovery(appdata, qb_mock, binary_path):
    """Le mock renvoie 403 (session expirée) sur le premier setPreferences,
    puis réussit. Le bridge doit re-login et appliquer le port.
    """
    qb_mock.configure_set_preferences_fail_count(1, status_code=403)
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a pas réussi à appliquer le port après perte de session"
        )

    assert qb_mock.last_set_port() == 63537, (
        f"Port attendu 63537 après récupération de session, obtenu {qb_mock.last_set_port()}"
    )


# ---------------------------------------------------------------------------
# Cas 10 : erreur temporaire qBittorrent -> retry non bloquant
# ---------------------------------------------------------------------------

def test_temporary_qbittorrent_error_retry(appdata, qb_mock, binary_path):
    """Le mock renvoie 500 sur le premier setPreferences, puis 200.
    Le bridge doit retry sans bloquer et finir par réussir.
    """
    qb_mock.configure_set_preferences_fail_count(1, status_code=500)
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a pas retry après une erreur temporaire qBittorrent"
        )

    assert qb_mock.last_set_port() == 63537, (
        f"Port attendu 63537 après retry, obtenu {qb_mock.last_set_port()}"
    )


# ---------------------------------------------------------------------------
# Cas 11 : logs avec bruit -> parsing robuste
# ---------------------------------------------------------------------------

def test_logs_with_noise_lines(appdata, qb_mock, binary_path):
    """Le fichier de logs contient beaucoup de lignes non-Proton autour
    du mapping valide. Le bridge doit ignorer le bruit et extraire le port.
    """
    noise_lines = "\n".join(
        f"2026-11-17T19:53:58.{i:03d}Z | INFO | OTHER.PROCESS | ligne de bruit {i} | {{}}"
        for i in range(50)
    )
    log_content = (
        noise_lines
        + "\n"
        + LOG_LINE_ACTIVE_63537
        + "\n"
        + noise_lines
    )
    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, log_content)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        assert _wait_for_set_port(qb_mock, 1), (
            "Le binaire n'a pas extrait le port au milieu du bruit"
        )

    assert qb_mock.last_set_port() == 63537, (
        f"Port attendu 63537 malgré le bruit, obtenu {qb_mock.last_set_port()}"
    )


# ---------------------------------------------------------------------------
# Cas 12 : port déjà correct -> aucun appel setPreferences inutile
# ---------------------------------------------------------------------------

def test_port_already_correct_no_unnecessary_change(appdata, qb_mock, binary_path):
    """Le port qBittorrent est déjà celui du mapping Proton.
    Aucun setPreferences ne doit être appelé (idempotence).
    """
    # Le mock démarre avec current_port = 6881. On le force à 63537
    # pour simuler un port déjà correct.
    qb_mock.state["current_port"] = 63537

    log_path = str(Path(appdata) / "proton" / "logs" / "client-logs.txt")
    write_proton_logs(log_path, LOG_LINE_ACTIVE_63537)
    write_config(
        appdata,
        url=f"http://127.0.0.1:{qb_mock.port}",
        log_path=log_path,
    )

    with launch_bridge(binary_path, appdata):
        time.sleep(WAIT_SECONDS)

    assert qb_mock.set_port_count() == 0, (
        "Aucun setPreferences ne doit être appelé quand le port est déjà correct"
    )
