#!/usr/bin/env python3
"""Prepare an isolated crate containing byte-identical s3 source/tests/fixtures.

Avoids rebuilding unrelated tine-store modules for every mutation. The final
gate still runs the real package. All products live under scratch/ and target/.
"""
import hashlib
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[2]
SOURCES = [f"src/page_state/{name}.rs" for name in ("mod", "operations", "order", "pairing", "saving")]


def source_sha256():
    digest = hashlib.sha256()
    for name in SOURCES:
        digest.update(name.encode() + b"\0")
        digest.update((ROOT / "crates/tine-store" / name).read_bytes())
    return digest.hexdigest()


def prepare(work=None):
    work = work or ROOT / "scratch/s3/mutation-harness"
    fixtures = ["scenarios", "traces", "witnesses", "audit-1019-twin-then-delete", "lane-1020-continuation-prefixes"]
    files = SOURCES + ["src/page_state/tests.rs"] + [f"tests/fixtures/s2/{name}.json" for name in fixtures]
    for name in files:
        source = ROOT / "crates/tine-store" / name
        dest = work / name
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, dest)
        assert hashlib.sha256(dest.read_bytes()).digest() == hashlib.sha256(source.read_bytes()).digest()
    (work / "src/lib.rs").write_text("mod page_state;\n")
    (work / "Cargo.toml").write_text('''[package]
name = "tine-store"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
serde = { version = "=1.0.228", features = ["derive"] }
serde_json = "=1.0.150"
[profile.dev]
debug = 0
''')
    print(work)
    return work


if __name__ == "__main__":
    prepare()
