# SPDX-License-Identifier: AGPL-3.0-or-later
"""Shared fixtures for the modelwrite Python bindings tests.

The live tests start the real `mw-server` binary on a scratch MW_DB and seed a
model over the HTTP write path. They are skipped, never failed, when no server
binary is available, so the always-on unit tests still run on a machine without a
Rust toolchain.
"""

from __future__ import annotations

import json
import os
import shutil
import socket
import subprocess
import sys
import time
import uuid
from pathlib import Path

import pytest
import requests

# The in-tree package must be importable however pytest was invoked.
PYTHON_DIR = Path(__file__).resolve().parents[1]
REPO_ROOT = PYTHON_DIR.parents[1]
MODELS_DIR = REPO_ROOT / "e2e" / "models"
TESTS_DIR = Path(__file__).resolve().parent
for _path in (PYTHON_DIR, TESTS_DIR):
    if str(_path) not in sys.path:
        sys.path.insert(0, str(_path))

SERVER_START_TIMEOUT = 30.0

#: The token the token-protected fixture server requires.
TEST_TOKEN = "python-bindings-secret"

#: The e2e models that commit standalone. cafe-stand.json is deliberately absent:
#: it pins cross-project revisions (purchasing-terminal, coffee-machine,
#: sandwich-toaster, floor-robot) that no longer resolve, so the server refuses it
#: with 422. See test_live_server.test_cafe_stand_needs_its_referenced_projects.
SELF_CONTAINED_MODELS = [
    "purchasing-terminal",
    "sandwich-toaster",
    "floor-robot",
    "microduck",
]


def load_model(name):
    """Load one e2e model document (the OKF root the commit endpoint expects)."""
    return json.loads((MODELS_DIR / (name + ".json")).read_text(encoding="utf-8"))


def _server_binary():
    """The mw-server executable: MW_SERVER_BIN, else a local cargo target, else PATH."""
    override = os.environ.get("MW_SERVER_BIN")
    candidates = []
    if override:
        candidates.append(Path(override))
    names = ["mw-server.exe", "mw-server"] if os.name == "nt" else ["mw-server", "mw-server.exe"]
    for directory in (
        "target-tests/debug",
        "target/debug",
        "target-tests/release",
        "target/release",
    ):
        for name in names:
            candidates.append(REPO_ROOT / directory / name)
    found = shutil.which("mw-server")
    if found:
        candidates.append(Path(found))
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def _free_port():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


class LocalServer:
    """A running mw-server (or an already-running one named by MW_TEST_BASE_URL)."""

    def __init__(self, base_url, process=None, log=None):
        self.base_url = base_url.rstrip("/")
        self._process = process
        self._log = log

    def get(self, path, **params):
        return requests.get(self.base_url + path, params=params, timeout=30)

    def post(self, path, body):
        return requests.post(self.base_url + path, json=body, timeout=60)

    def create_project(self, name):
        response = self.post("/projects", {"name": name})
        assert response.status_code == 201, (response.status_code, response.text)
        return name

    def commit_model(self, project, model, branch="main"):
        """Commit an OKF document through the same HTTP write path every model uses."""
        return self.post(
            "/projects/{}/commits".format(project),
            {
                "branch": branch,
                "author": "python-bindings-test",
                "message": "seed " + project,
                "okf": model,
            },
        )

    def seed(self, model_name, project=None):
        """Create a project and commit the named e2e model; returns (project, model)."""
        model = load_model(model_name)
        project = project or "{}-{}".format(model_name, uuid.uuid4().hex[:8])
        self.create_project(project)
        response = self.commit_model(project, model)
        assert response.status_code == 201, (response.status_code, response.text)
        return project, model

    def stop(self):
        if self._process is not None:
            self._process.terminate()
            try:
                self._process.wait(timeout=10)
            except subprocess.TimeoutExpired:
                self._process.kill()
                self._process.wait(timeout=10)
        if self._log is not None:
            self._log.close()


def _start_server(tmp_path, token=None):
    """Start a scratch server, or point at MW_TEST_BASE_URL when it is set."""
    base_url = os.environ.get("MW_TEST_BASE_URL")
    if base_url:
        return LocalServer(base_url)

    binary = _server_binary()
    if binary is None:
        pytest.skip(
            "no mw-server binary found: set MW_SERVER_BIN, build target/debug/mw-server, "
            "or set MW_TEST_BASE_URL to an already-running server"
        )

    port = _free_port()
    env = dict(os.environ)
    env["MW_DB"] = str(tmp_path / "scratch.db")
    env["MW_PORT"] = str(port)
    env["MW_EVIDENCE_DIR"] = str(tmp_path / "evidence")
    env["MW_BIND"] = "127.0.0.1"
    env.pop("MW_DATABASE_URL", None)
    if token:
        env["MW_AUTH_TOKEN"] = token
    else:
        env.pop("MW_AUTH_TOKEN", None)

    log_path = tmp_path / "mw-server.log"
    log = open(str(log_path), "wb")
    process = subprocess.Popen([str(binary)], env=env, stdout=log, stderr=subprocess.STDOUT)
    server = LocalServer("http://127.0.0.1:" + str(port), process, log)

    deadline = time.time() + SERVER_START_TIMEOUT
    while time.time() < deadline:
        if process.poll() is not None:
            log.flush()
            raise RuntimeError(
                "mw-server exited early:\n"
                + log_path.read_text(encoding="utf-8", errors="replace")
            )
        try:
            requests.get(server.base_url + "/health", timeout=1)
            return server
        except requests.RequestException:
            time.sleep(0.1)

    server.stop()
    raise RuntimeError(
        "mw-server did not become healthy within {}s".format(SERVER_START_TIMEOUT)
    )


@pytest.fixture(scope="session")
def mw_server(tmp_path_factory):
    """A scratch mw-server for the whole session (open mode, loopback)."""
    server = _start_server(tmp_path_factory.mktemp("mw-server"))
    yield server
    server.stop()


@pytest.fixture(scope="session")
def mw_server_with_token(tmp_path_factory):
    """A scratch mw-server that requires a bearer token (MW_AUTH_TOKEN)."""
    server = _start_server(tmp_path_factory.mktemp("mw-server-token"), token=TEST_TOKEN)
    yield server
    server.stop()


@pytest.fixture
def client(mw_server):
    """A modelwrite Client bound to the scratch server."""
    from modelwrite import Client

    return Client(base_url=mw_server.base_url)


@pytest.fixture
def seed(mw_server):
    """seed("purchasing-terminal") -> project name, committed to main."""

    def _seed(model_name, project=None):
        return mw_server.seed(model_name, project=project)

    return _seed
