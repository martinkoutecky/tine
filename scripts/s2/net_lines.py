#!/usr/bin/env python3
"""Net production and test lines for storage steps 2+3 (STEP3-DESIGN §12).

Counts non-blank, non-comment lines of every code file that differs between
the fixed baseline (default `9a0ebf01b`) and a target revision (default HEAD,
or the working tree with `--worktree`), and prints production and test totals
plus per-file deltas. The metric and its exclusions are identical on both
sides and must not change after the first measurement:

* code files only: `.rs`, `.ts`, `.tsx`, `.js`, `.mjs`, `.py`;
* test (never production): `#[cfg(test)]`-gated Rust items and modules,
  including embedded `mod x { ... }` blocks and files declared through a
  test-gated `mod x;`; items gated by a cfg naming `test` (for example
  `any(test, feature = "test-faults")`); `*_tests.rs`; anything under a
  `tests/` directory; `*.test.*`; everything under `scripts/`; and the
  test-only host oracle `crates/tine-store/src/page_state/`;
* comments: lines starting with `//`, `*`, or inside `/* ... */`.

Usage: scripts/s2/net_lines.py [--base REV] [--rev REV | --worktree]
"""
import argparse
import os
import re
import subprocess
import sys

CODE = (".rs", ".ts", ".tsx", ".js", ".mjs", ".py")
TEST_CFG = re.compile(r"#\[cfg\((?P<body>.*)\)\]\s*$")
MOD_DECL = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?mod\s+(\w+)\s*;")
PATH_ATTR = re.compile(r'^\s*#\[path\s*=\s*"([^"]+)"\]')


def git(*args):
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout


def read(rev, path):
    if rev is None:
        try:
            with open(path, encoding="utf-8", errors="replace") as f:
                return f.read()
        except FileNotFoundError:
            return None
    try:
        return git("show", f"{rev}:{path}")
    except subprocess.CalledProcessError:
        return None


def files_at(rev):
    if rev is None:
        tracked = git("ls-files").split("\n")
        untracked = git("ls-files", "--others", "--exclude-standard").split("\n")
        return [p for p in tracked + untracked if p and os.path.exists(p)]
    return [p for p in git("ls-tree", "-r", "--name-only", rev).split("\n") if p]


def is_test_cfg(line):
    m = TEST_CFG.match(line.strip())
    if not m:
        return False
    body = m.group("body")
    return re.search(r"\btest\b", body) is not None and "not(test" not in body


def test_path(path):
    name = os.path.basename(path)
    return (
        path.startswith("scripts/")
        or "/tests/" in f"/{path}"
        or name.endswith("_tests.rs")
        or ".test." in name
        or path.startswith("crates/tine-store/src/page_state/")
    )


def declared_test_modules(rev):
    """Rust files reached only through a test-gated `mod x;` declaration,
    or declared (gated or not) from such a file, transitively."""
    result = set()
    rust = [p for p in files_at(rev) if p.endswith(".rs")]
    texts = {p: read(rev, p) for p in rust}
    while True:
        found = set(result)
        for path in rust:
            found |= declared_from(path, texts[path], path in result or test_path(path))
        if found == result:
            return result
        result = found


def declared_from(path, text, all_test):
    result = set()
    if text is None:
        return result
    lines = text.split("\n")
    here = os.path.dirname(path)
    stem = os.path.basename(path)[:-3]
    mod_dir = here if stem in ("mod", "lib", "main") else os.path.join(here, stem)
    for i, line in enumerate(lines):
        m = MOD_DECL.match(line)
        if not m:
            continue
        gated, custom = all_test, None
        j = i - 1
        while j >= 0 and lines[j].strip().startswith("#["):
            gated |= is_test_cfg(lines[j])
            p = PATH_ATTR.match(lines[j])
            if p:
                custom = p.group(1)
            j -= 1
        if not gated:
            continue
        if custom:
            result.add(os.path.normpath(os.path.join(here, custom)))
        else:
            result.add(os.path.join(mod_dir, m.group(3) + ".rs"))
            result.add(os.path.join(mod_dir, m.group(3), "mod.rs"))
    return result


def strip_strings(line):
    """Drop string and char literal contents so braces inside them are inert."""
    out, i, n = [], 0, len(line)
    while i < n:
        c = line[i]
        if c == '"':
            i += 1
            while i < n and line[i] != '"':
                i += 2 if line[i] == "\\" else 1
            i += 1
            out.append('""')
        elif c == "'" and re.match(r"'(\\.|[^\\'])'", line[i:]):
            i += len(re.match(r"'(\\.|[^\\'])'", line[i:]).group(0))
            out.append("' '")
        elif line.startswith("//", i):
            break
        else:
            out.append(c)
            i += 1
    return "".join(out)


def count(text, rust):
    """(production, test) non-blank non-comment lines."""
    prod = test = 0
    in_block = False
    gate_depth = None  # brace depth at which a test-gated item ends
    pending_gate = False
    depth = 0
    for raw in text.split("\n"):
        line = raw.strip()
        if in_block:
            if "*/" in line:
                in_block = False
                line = line.split("*/", 1)[1].strip()
            else:
                continue
        if line.startswith("/*"):
            if "*/" not in line:
                in_block = True
            continue
        if not line or line.startswith("//") or line.startswith("*"):
            continue
        code = strip_strings(line) if rust else line
        starts_gate = rust and gate_depth is None and is_test_cfg(line)
        in_test = gate_depth is not None or pending_gate or starts_gate
        if in_test:
            test += 1
        else:
            prod += 1
        if not rust:
            continue
        opens, closes = code.count("{"), code.count("}")
        if starts_gate:
            pending_gate = True
            continue
        if pending_gate and not line.startswith("#["):
            pending_gate = False
            if opens > closes:
                gate_depth = depth
            # A one-line item (`fn f() {}` / `mod x;` / field) ends here.
        depth += opens - closes
        if gate_depth is not None and depth <= gate_depth:
            gate_depth = None
    return prod, test


def tally(rev, paths, test_mods):
    result = {}
    for path in paths:
        if not path.endswith(CODE):
            continue
        text = read(rev, path)
        if text is None:
            result[path] = (0, 0)
            continue
        rust = path.endswith(".rs")
        if path.endswith(".py"):
            text = "\n".join(
                l for l in text.split("\n") if not l.strip().startswith("#")
            )
        prod, test = count(text, rust)
        if test_path(path) or path in test_mods:
            prod, test = 0, prod + test
        result[path] = (prod, test)
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--base", default="9a0ebf01b")
    target = parser.add_mutually_exclusive_group()
    target.add_argument("--rev", default="HEAD")
    target.add_argument("--worktree", action="store_true")
    args = parser.parse_args()
    root = git("rev-parse", "--show-toplevel").strip()
    os.chdir(root)
    head = None if args.worktree else args.rev
    if head is None:
        changed = git("diff", "--no-renames", "--name-only", args.base).split("\n")
        changed += git("ls-files", "--others", "--exclude-standard").split("\n")
    else:
        changed = git("diff", "--no-renames", "--name-only", args.base, head).split("\n")
    changed = sorted({p for p in changed if p})
    before = tally(args.base, changed, declared_test_modules(args.base))
    after = tally(head, changed, declared_test_modules(head))
    total = [0, 0, 0, 0]
    rows = []
    for path in sorted(set(before) | set(after)):
        bp, bt = before.get(path, (0, 0))
        ap, at = after.get(path, (0, 0))
        total = [total[0] + bp, total[1] + ap, total[2] + bt, total[3] + at]
        if (bp, bt) != (ap, at):
            rows.append(f"  {path}: prod {ap - bp:+d} test {at - bt:+d}")
    label = "worktree" if head is None else head
    print(
        f"{args.base}..{label}: production {total[1] - total[0]:+d} "
        f"({total[0]} -> {total[1]} in changed files), "
        f"test {total[3] - total[2]:+d} ({total[2]} -> {total[3]})"
    )
    print("\n".join(rows))


if __name__ == "__main__":
    sys.exit(main())
