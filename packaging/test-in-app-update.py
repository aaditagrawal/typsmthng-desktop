#!/usr/bin/env python3
"""Run the GTK update flow against a private fake AppImage installation under Xvfb."""
import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path


def wait_until(condition, description):
    deadline = time.monotonic() + 12
    while not condition():
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Timed out waiting for {description}")
        time.sleep(0.02)


def run(app_path):
    with tempfile.TemporaryDirectory(prefix="typsmthng-update-ui-") as directory:
        root = Path(directory)
        marker = root / "relaunched"
        target = root / "installed.AppImage"
        original = b'#!/bin/sh\nprintf old > "$TYPSMTHNG_UPDATE_SMOKE_MARKER"\n'
        payload = b'#!/bin/sh\nprintf updated > "$TYPSMTHNG_UPDATE_SMOKE_MARKER"\n'
        target.write_bytes(original)
        target.chmod(0o755)
        requests = []
        name = "typsmthng-99.0.0-linux-x64.AppImage"
        checksum = hashlib.sha256(payload).hexdigest()

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: object) -> None:
                pass

            def do_GET(self):
                requests.append(self.path)
                if self.path == "/app":
                    body = payload
                elif self.path == "/sums":
                    body = f"{checksum}  {name}\n".encode()
                else:
                    body = json.dumps({"tag_name": "v99.0.0", "html_url": "https://github.com/aaditagrawal/typsmthng-desktop/releases/tag/v99.0.0", "body": "- A verified GUI update.", "assets": [
                        {"name": name, "browser_download_url": f"{base}/app"},
                        {"name": "SHA256SUMS", "browser_download_url": f"{base}/sums"},
                    ]}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

        server = HTTPServer(("127.0.0.1", 0), Handler)
        base = f"http://127.0.0.1:{server.server_port}"
        threading.Thread(target=server.serve_forever, daemon=True).start()
        environment = dict(os.environ, APPIMAGE=str(target), XDG_DATA_HOME=str(root / "data"),
                           XDG_CACHE_HOME=str(root / "cache"), TYPSMTHNG_SMOKE_HOLD_MS="25000",
                           TYPSMTHNG_UPDATE_SMOKE_MARKER=str(marker),
                           TYPSMTHNG_UPDATE_API_URL=f"http://127.0.0.1:{server.server_port}/latest")
        artifacts = Path.cwd() / "build/in-app-update-smoke"
        artifacts.mkdir(parents=True, exist_ok=True)
        (artifacts / "failure.png").unlink(missing_ok=True)
        log_path = artifacts / "application.log"
        with log_path.open("w") as log:
            app = subprocess.Popen([str(app_path), "--smoke-test"], env=environment, stdout=log, stderr=log)
            try:
                window = []

                def find_window():
                    if app.poll() is not None:
                        raise RuntimeError("The isolated app exited before opening a window")
                    found = subprocess.run(["xdotool", "search", "--onlyvisible", "--pid", str(app.pid), "--name", "^typsmthng$"], capture_output=True, text=True, check=False)
                    window[:] = found.stdout.splitlines()
                    return bool(window)

                wait_until(find_window, "the isolated application window")
                identifier = window[0]
                geometry = subprocess.check_output(["xdotool", "getwindowgeometry", "--shell", identifier], text=True)
                width = int(dict(line.split("=", 1) for line in geometry.splitlines())["WIDTH"])
                subprocess.run(["xdotool", "windowfocus", "--sync", identifier], check=True)
                subprocess.run(["xdotool", "mousemove", "--window", identifier, str(width - 210), "26", "click", "1"], check=True)
                wait_until(lambda: len(requests) == 1, "the manual metadata check")
                time.sleep(0.6)
                subprocess.run(["xdotool", "key", "Return"], check=True)
                wait_until(lambda: len(requests) == 3, "the installer and checksum download")
                assert requests == ["/latest", "/app", "/sums"], requests
                assert target.read_bytes() == original, "download installed before restart"
                time.sleep(0.6)
                subprocess.run(["xdotool", "key", "Return"], check=True)
                assert app.wait(timeout=12) == 0, "application failed to exit for restart"
                wait_until(lambda: marker.exists() and marker.read_text() == "updated", "the updated application relaunch")
                assert marker.read_text() == "updated", "the previous app was relaunched"
                assert target.read_bytes() == payload, "the installation was not replaced"
                print("In-app update check, verified download, restart, replacement, and relaunch passed")
            except Exception as error:
                if window:
                    try:
                        subprocess.run(["import", "-window", window[0], str(artifacts / "failure.png")], check=False, timeout=3)
                    except (FileNotFoundError, subprocess.TimeoutExpired):
                        pass
                raise RuntimeError(f"{error}\n{log_path.read_text()}") from error
            finally:
                if app.poll() is None:
                    app.terminate()
                    try:
                        app.wait(timeout=2)
                    except subprocess.TimeoutExpired:
                        app.kill()
                        app.wait()
                server.shutdown()
                server.server_close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    args = parser.parse_args()
    if sys.platform != "linux":
        parser.error("This validation uses Linux AppImage updates and Xvfb")
    app_path = args.app.resolve()
    if not app_path.is_file() or not app_path.with_name("typsmthng-updater").is_file():
        parser.error("Build the application and updater together with cargo build first")
    run(app_path)
