#!/usr/bin/env python3
"""Test-only TLS git-http-backend fixture, adapted from the ADR 0078 spike.

Python, git and the openssl CLI are test prerequisites, never app dependencies.
Logs contain request metadata, never the Authorization value or request bodies.
"""
import base64
import json
import os
from pathlib import Path
import ssl
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def certificates(directory):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    def run(*args):
        subprocess.run(["openssl", *args], cwd=directory, check=True,
                       stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    run("req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", "ca.key",
        "-out", "ca.pem", "-days", "2", "-subj", "/CN=Tine transport test CA",
        "-addext", "basicConstraints=critical,CA:TRUE",
        "-addext", "keyUsage=critical,keyCertSign,cRLSign")
    run("req", "-newkey", "rsa:2048", "-nodes", "-keyout", "leaf.key",
        "-out", "leaf.csr", "-subj", "/CN=localhost")
    (directory / "ext.cnf").write_text(
        "subjectAltName=DNS:localhost,IP:127.0.0.1\n"
        "basicConstraints=CA:FALSE\nkeyUsage=digitalSignature,keyEncipherment\n"
        "extendedKeyUsage=serverAuth\n")
    run("x509", "-req", "-in", "leaf.csr", "-CA", "ca.pem", "-CAkey", "ca.key",
        "-CAcreateserial", "-out", "leaf.pem", "-days", "2", "-extfile", "ext.cnf")


if sys.argv[1] == "--certs":
    certificates(sys.argv[2])
    sys.exit(0)

root, certs, log_path, mode, destination = sys.argv[1:]
log_lock = threading.Lock()


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *args):
        pass

    def record(self, status):
        with log_lock, open(log_path, "a") as log:
            log.write(json.dumps({"method": self.command, "path": self.path,
                                  "auth": bool(self.headers.get("Authorization")),
                                  "status": status}) + "\n")

    def answer(self, status, headers=(), body=b""):
        self.record(status)
        self.send_response(status)
        for key, value in headers:
            self.send_header(key, value)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        try:
            self.wfile.write(body)
        except (BrokenPipeError, ConnectionResetError, ssl.SSLError):
            pass  # cancellation deliberately closes the client socket

    def handle_git(self):
        path, _, query = self.path.partition("?")
        body = self.rfile.read(int(self.headers.get("Content-Length", "0")))
        auth = self.headers.get("Authorization", "")
        good = auth == "Basic " + base64.b64encode(b"token-user:fixture-token").decode()
        if (mode in ("auth", "auth-redirect") or
                mode == "auth-post" and self.command == "POST") and not good:
            return self.answer(401, [("WWW-Authenticate", 'Basic realm="git"')])
        if path.startswith("/old/"):
            return self.answer(301, [("Location", "/" + self.path[5:])])
        if mode in ("redirect", "auth-redirect"):
            return self.answer(302, [("Location", destination + self.path)])
        if mode == "loop":
            return self.answer(307, [("Location", self.path)])
        if mode == "post-redirect" and self.command == "POST":
            return self.answer(307, [("Location", destination + self.path)])
        if mode in ("stall-fetch", "stall-push") and path.endswith(
                "git-upload-pack" if mode == "stall-fetch" else "git-receive-pack"):
            self.record(102)  # test synchronizes cancellation with a live request
            time.sleep(10)
        if (mode == "hold-fetch" and path.endswith("git-upload-pack") or
                mode == "hold-ls" and self.command == "GET"):
            self.record(102)
            deadline = time.monotonic() + 5
            while not (Path(root) / "release-request").exists():
                if time.monotonic() >= deadline:
                    return self.answer(504)
                time.sleep(0.01)
        if mode == "bad-type":
            return self.answer(200, [("Content-Type", "text/html")], b"not git")
        env = dict(os.environ, GIT_PROJECT_ROOT=root, GIT_HTTP_EXPORT_ALL="1",
                   REQUEST_METHOD=self.command, PATH_INFO=path, QUERY_STRING=query,
                   CONTENT_TYPE=self.headers.get("Content-Type", ""),
                   CONTENT_LENGTH=str(len(body)), SERVER_PROTOCOL="HTTP/1.1",
                   REMOTE_ADDR="127.0.0.1", REMOTE_USER="fixture")
        result = subprocess.run(["git", "http-backend"], input=body, env=env,
                                capture_output=True, timeout=20)
        headers, separator, payload = result.stdout.partition(b"\r\n\r\n")
        if not separator:
            headers, _, payload = result.stdout.partition(b"\n\n")
        status, output_headers = 200, []
        for line in headers.decode("latin1").splitlines():
            key, _, value = line.partition(":")
            if key.lower() == "status":
                status = int(value.strip().split()[0])
            else:
                output_headers.append((key, value.strip()))
        self.answer(status, output_headers, payload)

    do_GET = do_POST = handle_git


server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
context.load_cert_chain(str(Path(certs) / "leaf.pem"), str(Path(certs) / "leaf.key"))
server.socket = context.wrap_socket(server.socket, server_side=True)
print("READY", server.server_address[1], flush=True)
server.serve_forever()
