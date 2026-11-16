import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';
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

const crates = ['crates/tine-core', 'crates/tine-store', 'crates/tine-graph-features', 'crates/tine-outline', 'src-tauri'];

for (const crate of crates) {
  test(`${crate}: one integration target includes every test module`, () => {
    const manifest = readFileSync(`${crate}/Cargo.toml`, 'utf8');
    assert.match(manifest, /^autotests = false$/m);
    assert.equal((manifest.match(/^\[\[test\]\]$/gm) ?? []).length, 1);
    assert.match(manifest, /name = "integration"\npath = "tests\/integration.rs"/);
    const entry = readFileSync(`${crate}/tests/integration.rs`, 'utf8');
    const fromLib = new Set(libIncludedTests(`${crate}/src`));
    const files = readdirSync(`${crate}/tests`).filter(
      name => name.endsWith('.rs') && name !== 'integration.rs' && !fromLib.has(name),
    );
    const modules = [...entry.matchAll(/^mod (\w+);$/gm)].map(match => `${match[1]}.rs`);
    assert.deepEqual(modules.sort(), files.sort());
    // A suite compiled into both targets would run twice.
    assert.deepEqual(modules.filter(name => fromLib.has(name)), []);
  });
}

test('third-party debug info is disabled while workspace line tables remain', () => {
  const manifest = readFileSync('Cargo.toml', 'utf8');
  assert.match(manifest, /\[profile.dev\]\ndebug = "line-tables-only"/);
  assert.match(manifest, /\[profile.dev.package."\*"\]\ndebug = false/);
});
