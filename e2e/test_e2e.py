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
"""

from __future__ import annotations

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

    import subprocess

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
