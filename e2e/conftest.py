"""Configuration et fixtures partagées pour les tests E2E de Proton Bridge.

Lance le binaire Rust compilé en Release avec un ``APPDATA`` temporaire et un
mock qBittorrent en Python. Chaque test peut écrire son propre ``config.toml``
et ses propres fichiers de logs Proton pour simuler divers scénarios.
"""

from __future__ import annotations

import os
import subprocess
import time
from contextlib import contextmanager
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Generator

import pytest

from mock_qbittorrent import QBittorrentMock


# ---------------------------------------------------------------------------
# Fixtures d'infrastructure
# ---------------------------------------------------------------------------

@pytest.fixture
def appdata(tmp_path: Path) -> Generator[str, None, None]:
    """Fournit un répertoire APPDATA isolé pour la durée du test."""
    appdata_dir = tmp_path / "appdata"
    appdata_dir.mkdir(parents=True, exist_ok=True)
    old_appdata = os.environ.get("APPDATA")
    os.environ["APPDATA"] = str(appdata_dir)
    yield str(appdata_dir)
    if old_appdata is None:
        os.environ.pop("APPDATA", None)
    else:
        os.environ["APPDATA"] = old_appdata


@pytest.fixture
def qb_mock() -> Generator[QBittorrentMock, None, None]:
    """Lance un mock qBittorrent sur un port libre et l'arrête après le test."""
    mock = QBittorrentMock()
    mock.start()
    yield mock
    mock.stop()


@pytest.fixture
def binary_path() -> str:
    """Chemin vers le binaire Release de Proton Bridge."""
    candidates = [
        Path("target/release/proton-bridge.exe"),
        Path("target/release/proton-bridge"),
    ]
    for candidate in candidates:
        if candidate.exists():
            return str(candidate.resolve())
    pytest.fail(
        "Binaire Release introuvable. Lancez `cargo build --release` avant les tests E2E."
    )


# ---------------------------------------------------------------------------
# Helpers d'écriture de configuration / logs
# ---------------------------------------------------------------------------

def write_config(
    appdata: str,
    url: str,
    username: str = "admin",
    password: str = "test",
    log_path: str = "",
    poll_interval: int = 1,
) -> Path:
    """Écrit un config.toml dans le répertoire APPDATA du test."""
    config_dir = Path(appdata) / "proton-bridge"
    config_dir.mkdir(parents=True, exist_ok=True)
    # Échapper les backslashes Windows pour le TOML (ex: C:\Users -> C:\\Users)
    log_path_escaped = log_path.replace("\\", "\\\\")
    config_content = f"""[qbittorrent]
url = "{url}"
username = "{username}"
password = "{password}"

[proton]
log_path = "{log_path_escaped}"

[general]
log_level = "debug"
poll_interval_seconds = {poll_interval}
auto_update = true
check_interval_hours = 24
"""
    config_file = config_dir / "config.toml"
    config_file.write_text(config_content, encoding="utf-8")
    return config_file


def write_proton_logs(log_path: str, content: str) -> None:
    """Écrit le contenu des logs Proton dans le fichier cible."""
    path = Path(log_path)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content, encoding="utf-8")


@contextmanager
def launch_bridge(binary_path: str, appdata: str) -> Generator[subprocess.Popen, None, None]:
    """Lance le binaire Rust et le termine proprement à la sortie du bloc."""
    env = os.environ.copy()
    env["APPDATA"] = appdata
    proc = subprocess.Popen([binary_path], env=env)
    try:
        yield proc
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()


# ---------------------------------------------------------------------------
# Générateur de lignes de logs Proton avec date d'expiration dans le futur
# ---------------------------------------------------------------------------

def _future_expiry(minutes_from_now: int = 10) -> str:
    """Retourne une date d'expiration au format Proton (DD/MM/YYYY HH:MM:SS)
    située dans le futur, pour éviter que le watcher ne considère le mapping
    comme expiré."""
    now = datetime.now(timezone.utc)
    expiry = now + timedelta(minutes=minutes_from_now)
    return expiry.strftime("%d/%m/%Y %H:%M:%S")


def make_active_log_line(port: int, minutes_from_now: int = 10) -> str:
    """Génère une ligne de log Proton avec un mapping actif sur le port donné,
    expirant dans le futur."""
    expiry = _future_expiry(minutes_from_now)
    triggered = datetime.now(timezone.utc).strftime("%d/%m/%Y %H:%M:%S")
    return (
        f"2026-11-17T19:53:58.706Z | INFO | PROCESS.COMM | "
        f"Received PortForwarding Status 'SleepingUntilRefresh' triggered at '{triggered}', "
        f"Port pair {port}->{port}, expiring in 00:01:00 at {expiry} | {{}}"
    )


def make_stopped_log_line() -> str:
    """Génère une ligne de log Proton avec statut Stopped."""
    triggered = datetime.now(timezone.utc).strftime("%d/%m/%Y %H:%M:%S")
    return (
        f"2026-11-17T19:55:00.000Z | INFO | PROCESS.COMM | "
        f"Received PortForwarding Status 'Stopped' triggered at '{triggered}' | {{}}"
    )


LOG_LINE_EXPIRED = (
    "2020-01-01T00:00:00.000Z | INFO | PROCESS.COMM | "
    "Received PortForwarding Status 'SleepingUntilRefresh' triggered at '01/01/2020 00:00:00', "
    "Port pair 63537->63537, expiring in 00:01:00 at 01/01/2020 00:01:00 | {}"
)

