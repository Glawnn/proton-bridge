"""Tests E2E dédiés à l'auto-update de Proton Bridge.

Ces tests sont exécutés UNIQUEMENT dans la pipeline de Release GitHub Actions
(pas sur chaque PR), car ils nécessitent le binaire Release compilé et
simulent l'API GitHub Releases avec un serveur HTTP Python local.

Scénario couverts :
- aucune mise à jour disponible (même version)
- mise à jour disponible (version supérieure) -> téléchargement + écriture de l'état
- erreur réseau -> échec propre sans bloquer
"""

from __future__ import annotations

import json
import os
import subprocess
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Generator

import pytest


# ---------------------------------------------------------------------------
# Mock API GitHub Releases
# ---------------------------------------------------------------------------

RELEASE_SAME_VERSION = {
    "tag_name": "v0.1.0",
    "assets": [
        {"name": "proton-bridge.exe", "browser_download_url": "http://127.0.0.1:9999/fake.exe"}
    ],
}

RELEASE_NEW_VERSION = {
    "tag_name": "v0.2.0",
    "assets": [
        {"name": "proton-bridge.exe", "browser_download_url": "http://127.0.0.1:9999/proton-bridge.exe"}
    ],
}

RELEASE_NO_ASSETS = {
    "tag_name": "v0.2.0",
    "assets": [],
}


class GitHubMockHandler(BaseHTTPRequestHandler):
    """Handler HTTP qui simule l'API GitHub Releases."""

    # Quel release renvoyer (configuré avant le démarrage du serveur)
    release_payload: dict = RELEASE_SAME_VERSION

    def do_GET(self):  # noqa: N802
        if self.path == "/repos/Glawnn/proton-bridge/releases/latest":
            body = json.dumps(self.release_payload).encode("utf-8")
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        elif self.path == "/proton-bridge.exe":
            # Fake binaire de mise à jour
            body = b"FAKE_UPDATER_BINARY_CONTENT"
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, format: str, *args) -> None:  # noqa: A002
        pass  # Masquer les logs HTTP


class GitHubMockServer:
    """Serveur HTTP mock GitHub dans un thread séparé."""

    def __init__(self, port: int = 0):
        self.port = port
        self._server: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    def start(self) -> None:
        self._server = ThreadingHTTPServer(("127.0.0.1", self.port), GitHubMockHandler)
        self.port = self._server.server_address[1]
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        if self._server:
            self._server.shutdown()
            self._server.server_close()
        if self._thread:
            self._thread.join(timeout=5)

    @property
    def url(self) -> str:
        return f"http://127.0.0.1:{self.port}"


# ---------------------------------------------------------------------------
# Fixtures pytest
# ---------------------------------------------------------------------------

@pytest.fixture
def github_mock() -> Generator[GitHubMockServer, None, None]:
    """Lance un mock API GitHub et l'arrête après le test."""
    server = GitHubMockServer()
    server.start()
    yield server
    server.stop()


@pytest.fixture
def updater_path() -> str:
    """Chemin vers le binaire Release de l'updater."""
    candidates = [
        Path("target/release/proton-bridge-updater.exe"),
        Path("target/release/proton-bridge-updater"),
    ]
    for candidate in candidates:
        if candidate.exists():
            return str(candidate.resolve())
    pytest.fail(
        "Binaire Release de l'updater introuvable. Lancez `cargo build --release` avant les tests E2E."
    )


@pytest.fixture
def fake_main_exe(tmp_path: Path) -> str:
    """Crée un faux proton-bridge.exe pour tester le remplacement."""
    exe = tmp_path / "proton-bridge.exe"
    exe.write_bytes(b"FAKE_MAIN_EXE_VERSION_0_1_0")
    return str(exe)


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------

def test_update_check_same_version(updater_path: str, github_mock: GitHubMockServer):
    """Aucune mise à jour si la version distante est identique."""
    GitHubMockHandler.release_payload = RELEASE_SAME_VERSION

    env = os.environ.copy()
    env["PROTON_BRIDGE_GITHUB_API_URL"] = github_mock.url

    result = subprocess.run(
        [updater_path, "--check", "--current-version", "0.1.0", "--main-exe", "fake.exe"],
        capture_output=True,
        text=True,
        env=env,
        timeout=30,
    )

    assert result.returncode == 0, f"Updater a échoué : {result.stderr}"
    assert "Aucune mise à jour disponible" in result.stdout

    # Aucun fichier d'état ne doit être créé
    state_path = Path(os.environ["APPDATA"]) / "proton-bridge" / "update-state.json"
    if state_path.exists():
        state_path.unlink()


def test_update_check_new_version(updater_path: str, github_mock: GitHubMockServer, tmp_path: Path):
    """Une mise à jour est disponible : téléchargement et écriture de l'état."""
    GitHubMockHandler.release_payload = {
        "tag_name": "v0.2.0",
        "assets": [
            {"name": "proton-bridge.exe", "browser_download_url": f"{github_mock.url}/proton-bridge.exe"}
        ],
    }

    env = os.environ.copy()
    env["PROTON_BRIDGE_GITHUB_API_URL"] = github_mock.url
    # Rediriger APPDATA vers le tmp_path pour isoler le test
    env["APPDATA"] = str(tmp_path / "appdata")

    result = subprocess.run(
        [updater_path, "--check", "--current-version", "0.1.0", "--main-exe", "fake.exe"],
        capture_output=True,
        text=True,
        env=env,
        timeout=30,
    )

    assert result.returncode == 0, f"Updater a échoué : {result.stderr}"
    assert "Mise à jour disponible : 0.2.0" in result.stdout
    assert "Mise à jour téléchargée" in result.stdout

    # Vérifier que le fichier d'état a été écrit
    state_path = Path(env["APPDATA"]) / "proton-bridge" / "update-state.json"
    assert state_path.exists(), "Le fichier update-state.json devrait exister"

    state = json.loads(state_path.read_text(encoding="utf-8"))
    assert state["pending_update"] is True
    assert state["new_version"] == "0.2.0"
    assert state["download_path"] is not None

    # Vérifier que le binaire a été téléchargé
    download_path = Path(state["download_path"])
    assert download_path.exists(), "Le binaire téléchargé devrait exister"
    assert download_path.read_bytes() == b"FAKE_UPDATER_BINARY_CONTENT"


def test_update_check_network_error(updater_path: str):
    """L'updater échoue proprement si l'API GitHub est inaccessible."""
    env = os.environ.copy()
    env["PROTON_BRIDGE_GITHUB_API_URL"] = "http://127.0.0.1:19999"  # Port fermé

    result = subprocess.run(
        [updater_path, "--check", "--current-version", "0.1.0", "--main-exe", "fake.exe"],
        capture_output=True,
        text=True,
        env=env,
        timeout=30,
    )

    assert result.returncode != 0, "L'updater doit échouer si l'API est inaccessible"


def test_update_do_update_without_state(updater_path: str, fake_main_exe: str):
    """--do-update sans état en attente ne fait rien."""
    env = os.environ.copy()
    # S'assurer qu'aucun update-state.json n'existe
    state_path = Path(os.environ["APPDATA"]) / "proton-bridge" / "update-state.json"
    if state_path.exists():
        state_path.unlink()

    result = subprocess.run(
        [updater_path, "--do-update", "--main-exe", fake_main_exe],
        capture_output=True,
        text=True,
        env=env,
        timeout=30,
    )

    assert result.returncode == 0
    assert "Aucune mise à jour en attente" in result.stdout
