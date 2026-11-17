"""Mock de l'API Web qBittorrent pour les tests E2E.

Reproduit les endpoints utilisés par Proton Bridge :
- POST /api/v2/auth/login  -> 204 ou "Ok."
- GET  /api/v2/app/preferences -> {"listen_port": <port>}
- POST /api/v2/app/setPreferences -> enregistre le port demandé

L'historique des appels est conservé dans l'objet ``state`` partagé, ce qui
permet aux tests pytest de vérifier que le binaire Rust a bien (ou pas)
modifié le port de qBittorrent.
"""

from __future__ import annotations

import json
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable
from urllib.parse import parse_qs


class QBittorrentMock:
    """Serveur HTTP mock qBittorrent exécuté dans un thread séparé."""

    def __init__(self, host: str = "127.0.0.1", port: int = 0):
        self.host = host
        self.port = port
        self.state: dict = {
            "login_calls": 0,
            "set_preferences_calls": [],  # liste des ports demandés
            "current_port": 6881,          # port "configuré" dans le mock
            "login_fail": False,
            "set_preferences_fail": False,
        }
        self._server: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    # -- cycle de vie -------------------------------------------------------
    def start(self) -> None:
        handler_factory = self._handler_factory()
        self._server = ThreadingHTTPServer((self.host, self.port), handler_factory)
        self.port = self._server.server_address[1]  # port réel si port=0
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        if self._server:
            self._server.shutdown()
            self._server.server_close()
        if self._thread:
            self._thread.join(timeout=5)

    # -- fabrication du handler avec état partagé ---------------------------
    def _handler_factory(self) -> Callable:
        state = self.state

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self) -> None:  # noqa: N802
                if self.path == "/api/v2/auth/login":
                    if state["login_fail"]:
                        self.send_response(403)
                        self.end_headers()
                        return
                    self.send_response(204)
                    self.end_headers()
                    state["login_calls"] += 1
                    return

                if self.path == "/api/v2/app/setPreferences":
                    length = int(self.headers.get("Content-Length", 0))
                    body = self.rfile.read(length).decode("utf-8")
                    params = parse_qs(body)
                    json_str = params.get("json", [""])[0]
                    try:
                        data = json.loads(json_str)
                        port = data.get("listen_port")
                    except json.JSONDecodeError:
                        port = None

                    if state["set_preferences_fail"] or port is None:
                        self.send_response(400)
                        self.end_headers()
                        return

                    state["set_preferences_calls"].append(port)
                    state["current_port"] = port
                    self.send_response(200)
                    self.end_headers()
                    return

                self.send_response(404)
                self.end_headers()

            def do_GET(self) -> None:  # noqa: N802
                if self.path == "/api/v2/app/preferences":
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    prefs = {"listen_port": state["current_port"]}
                    self.wfile.write(json.dumps(prefs).encode("utf-8"))
                    return

                self.send_response(404)
                self.end_headers()

            def log_message(self, format: str, *args) -> None:  # noqa: A002
                # Masquer les logs HTTP du mock pendant les tests.
                pass

        return Handler

    # -- helpers d'assertion ------------------------------------------------
    def last_set_port(self) -> int | None:
        calls = self.state["set_preferences_calls"]
        return calls[-1] if calls else None

    def set_port_count(self) -> int:
        return len(self.state["set_preferences_calls"])

    def reset_calls(self) -> None:
        self.state["set_preferences_calls"] = []
