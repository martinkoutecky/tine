import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { basename, dirname, join, normalize } from 'node:path';
import { test } from 'node:test';

// Suites that need crate-private API compile into the lib test target from
// `src/` (`#[path = "../tests/x.rs"]`, Storage S4); every other file in
// `tests/` belongs to the one integration target.
function libIncludedTests(dir) {
  const found = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) found.push(...libIncludedTests(path));
    else if (entry.name.endsWith('.rs')) {
      for (const match of readFileSync(path, 'utf8').matchAll(/#\[path = "(?:\.\.\/)+tests\/(\w+\.rs)"\]/g)) {
        found.push(match[1]);
      }
    }
  }
  return found;
}

// `mod x;` declarations at the start of a line (a commented-out one does not compile), with an optional `#[path]`.
const MODULE = /^[ \t]*(?:#\[path = "([^"]+)"\][ \t]*\n[ \t]*)?(?:pub(?:\([^)]*\))? )?mod (\w+);/gm;

// How many times each file is compiled into the integration target, following `mod` from `tests/integration.rs`.
function integrationModules(crate) {
  const root = `${crate}/tests/integration.rs`;
  const reached = new Map();
  const queue = [root];
  while (queue.length) {
    const file = queue.shift();
    for (const [, path, name] of readFileSync(file, 'utf8').matchAll(MODULE)) {
      const stem = file === root ? '' : `${basename(file, '.rs')}/`;
      const target = normalize(join(dirname(file), path ?? `${stem}${name}.rs`));
      reached.set(target, (reached.get(target) ?? 0) + 1);
      if (reached.get(target) === 1) queue.push(target);
    }
  }
  return reached;
}

// The `#[test]` functions of `source` that take neither integration lock, directly or through a helper they
// call. Bodies end at the closing brace at the function's own indentation (rustfmt).
function unlockedTests(source) {
  const body = name => {
    const start = source.search(new RegExp(`^([ \\t]*)(?:pub )?fn ${name}\\(`, 'm'));
    if (start < 0) return '';
    const indent = source.slice(start).match(/^[ \t]*/)[0];
    const end = source.indexOf(`\n${indent}}\n`, start);
    return source.slice(start, end < 0 ? undefined : end);
  };
  const locks = (name, depth) => {
    const text = body(name);
    if (/integration_lock\(\)/.test(text)) return true;
    if (depth === 0) return false;
    return [...text.slice(text.indexOf('{')).matchAll(/\b([a-z_][a-z0-9_]*)\(/g)].some(([, called]) => called !== name && locks(called, depth - 1));
  };
  const tests = [...source.matchAll(/^[ \t]*#\[test\][^\n]*\n(?:[ \t]*(?:#\[|\/\/)[^\n]*\n)*[ \t]*(?:pub )?fn (\w+)\(/gm)];
  return tests.map(([, name]) => name).filter(name => !locks(name, 1));
}

const crates = ['crates/tine-core', 'crates/tine-store', 'crates/tine-graph-features', 'crates/tine-outline', 'src-tauri'];

for (const crate of crates) {
  test(`${crate}: one integration target includes every test module`, () => {
    const manifest = readFileSync(`${crate}/Cargo.toml`, 'utf8');
    assert.match(manifest, /^autotests = false$/m);
    assert.equal((manifest.match(/^\[\[test\]\]$/gm) ?? []).length, 1);
    assert.match(manifest, /name = "integration"\npath = "tests\/integration.rs"/);
    const fromLib = new Set(libIncludedTests(`${crate}/src`));
    const reached = integrationModules(crate);
    const files = readdirSync(`${crate}/tests`).filter(name => name.endsWith('.rs') && name !== 'integration.rs');
    // A file with tests runs exactly once: in the lib target (Storage S4) or in the one integration target.
    // A helper without tests may be compiled into both, but into at least one.
    const runs = files.filter(name => {
      const count = (reached.get(`${crate}/tests/${name}`) ?? 0) + (fromLib.has(name) ? 1 : 0);
      return /^[ \t]*#\[test\]/m.test(readFileSync(`${crate}/tests/${name}`, 'utf8')) ? count !== 1 : count === 0;
    });
    assert.deepEqual(runs.map(name => `${name}: ${reached.get(`${crate}/tests/${name}`) ?? 0} + ${fromLib.has(name) ? 1 : 0}`), []);
    const entry = readFileSync(`${crate}/tests/integration.rs`, 'utf8');
    if (!/^static INTEGRATION_LOCK:/m.test(entry)) return;
    // A cost probe holds the exclusive lock, so every case beside it must hold the shared one.
    const unlocked = [];
    for (const file of reached.keys()) {
      if (!file.startsWith(`${crate}/tests/`)) continue;
      for (const name of unlockedTests(readFileSync(file, 'utf8'))) unlocked.push(`${file}: ${name}`);
    }
    assert.deepEqual(unlocked, []);
  });
}

test('the layout check counts every inclusion and sees an unlocked case', () => {
  const crate = mkdtempSync(join(tmpdir(), 'layout-'));
  mkdirSync(join(crate, 'tests'));
  const write = (name, text) => writeFileSync(join(crate, 'tests', name), text);
  write('integration.rs', '#[path = "a.rs"]\nmod a;\nmod b;\n// mod commented;\n');
  write('a.rs', '#[path = "shared.rs"]\nmod shared;\n');
  write('b.rs', '#[path = "shared.rs"]\nmod shared;\n');
  write('shared.rs', '');
  write('c.rs', '#[path = "d.rs"]\nmod d;\n');
  write('d.rs', '#[path = "c.rs"]\nmod c;\n');
  const reached = integrationModules(crate);
  rmSync(crate, { recursive: true });
  const count = name => reached.get(join(crate, 'tests', name)) ?? 0;
  assert.deepEqual(['a.rs', 'b.rs', 'shared.rs', 'c.rs', 'd.rs', 'commented.rs'].map(count), [1, 1, 2, 0, 0, 0]);
  const source = '#[test]\nfn locked() {\n    let _g = crate::integration_lock();\n}\n\n#[test]\n#[ignore]\nfn via_helper() {\n    helper();\n}\n\nfn helper() {\n    let _g = crate::exclusive_integration_lock();\n}\n\n#[test]\nfn bare() {\n    let x = 1;\n}\n';
  assert.deepEqual(unlockedTests(source), ['bare']);
});

test('third-party debug info is disabled while workspace line tables remain', () => {
  const manifest = readFileSync('Cargo.toml', 'utf8');
  assert.match(manifest, /\[profile.dev\]\ndebug = "line-tables-only"/);
  assert.match(manifest, /\[profile.dev.package."\*"\]\ndebug = false/);
});
