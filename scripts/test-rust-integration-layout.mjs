import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { test } from 'node:test';

const crates = ['crates/tine-core', 'crates/tine-store', 'crates/tine-graph-features', 'crates/tine-outline', 'src-tauri'];

for (const crate of crates) {
  test(`${crate}: one integration target includes every test module`, () => {
    const manifest = readFileSync(`${crate}/Cargo.toml`, 'utf8');
    assert.match(manifest, /^autotests = false$/m);
    assert.equal((manifest.match(/^\[\[test\]\]$/gm) ?? []).length, 1);
    assert.match(manifest, /name = "integration"\npath = "tests\/integration.rs"/);
    const entry = readFileSync(`${crate}/tests/integration.rs`, 'utf8');
    const files = readdirSync(`${crate}/tests`).filter(name => name.endsWith('.rs') && name !== 'integration.rs');
    const modules = [...entry.matchAll(/^mod (\w+);$/gm)].map(match => `${match[1]}.rs`);
    assert.deepEqual(modules.sort(), files.sort());
  });
}

test('third-party debug info is disabled while workspace line tables remain', () => {
  const manifest = readFileSync('Cargo.toml', 'utf8');
  assert.match(manifest, /\[profile.dev\]\ndebug = "line-tables-only"/);
  assert.match(manifest, /\[profile.dev.package."\*"\]\ndebug = false/);
});
