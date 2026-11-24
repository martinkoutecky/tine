#!/usr/bin/env python3
"""The storage model pins agree with the canonical model (conformance audit
2026-10-12, G8).

The Rust conformance tests check the committed fixtures against
`page_state::MODEL_SHA`, and the generators check the canonical model
against their own pins, but only when they run. A change to the canonical
model alone (`docs/storage-model/quint/storage-s35.qnt` or its scenarios) passes every test
on the old fixtures. This check closes that gap: run it after regenerating
and before integrating a storage packet.

Usage: check_pins.py [MODEL_DIR]   (default: docs/storage-model/quint)
"""
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(Path(__file__).parent))
from generate import SCENARIO_SHA, SHA  # noqa: E402


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_text().encode()).hexdigest()


def main() -> int:
    model_dir = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "docs/storage-model/quint"
    rust = (ROOT / "crates/tine-store/src/page_state/mod.rs").read_text()
    rust_sha = re.search(r'const MODEL_SHA: &str = "([0-9a-f]{64})";', rust).group(1)
    fixtures = ROOT / "crates/tine-store/tests/fixtures/s2"
    pins = {
        "canonical storage-s35.qnt": digest(model_dir / "storage-s35.qnt"),
        "page_state::MODEL_SHA": rust_sha,
    }
    for name in ("scenarios.json", "traces.json", "witnesses.json"):
        pins[f"fixture {name}"] = json.loads((fixtures / name).read_text())["model_sha256"]
    failures = [f"{name}: {sha}" for name, sha in pins.items() if sha != SHA]
    scenarios = {
        "canonical scenarios-s35.inc": digest(model_dir / "scenarios-s35.inc"),
        "fixture scenarios.json": json.loads((fixtures / "scenarios.json").read_text())["scenario_sha256"],
    }
    failures += [f"{name}: {sha}" for name, sha in scenarios.items() if sha != SCENARIO_SHA]
    if failures:
        print(f"model pins disagree (generate.py pins model {SHA}, scenarios {SCENARIO_SHA}):")
        for failure in failures:
            print(f"  {failure}")
        print("Regenerate (scripts/s2/generate.py, witnesses.py) and update the pins together.")
        return 1
    print(f"model pins agree: model {SHA[:12]}, scenarios {SCENARIO_SHA[:12]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
