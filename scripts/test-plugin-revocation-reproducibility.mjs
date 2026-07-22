#!/usr/bin/env node

import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sourceRoot = path.join(root, "fixtures/plugin-revocation/sentinel-src");
const committedWasm = path.join(root, "fixtures/plugin-revocation/plugin.wasm");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-revocation-repro-"));
const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");

function buildAt(label) {
  const crateRoot = path.join(temporary, label, "sentinel-src");
  fs.cpSync(sourceRoot, crateRoot, {
    recursive: true,
    filter: (source) => !source.split(path.sep).includes("target"),
  });
  const result = spawnSync("cargo", ["build", "--release", "--locked", "--offline"], {
    cwd: crateRoot,
    encoding: "utf8",
    env: process.env,
  });
  if (result.status !== 0) {
    throw new Error(`fixture build failed at ${crateRoot}:\n${result.stdout}\n${result.stderr}`);
  }
  return fs.readFileSync(path.join(
    crateRoot,
    "target/wasm32-unknown-unknown/release/tine_plugin_revocation_sentinel.wasm",
  ));
}

function assertRunnableAbi(bytes) {
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), [
    { module: "env", name: "memory", kind: "memory" },
  ]);
  const exportNames = WebAssembly.Module.exports(module).map(({ name }) => name);
  for (const name of ["tine_alloc", "tine_handle", "tine_result_len"]) {
    assert.ok(exportNames.includes(name), `fixture is missing ${name}`);
  }

  const memory = new WebAssembly.Memory({ initial: 32, maximum: 256 });
  const { exports } = new WebAssembly.Instance(module, { env: { memory } });
  const event = Buffer.from(JSON.stringify({ protocolVersion: 2, kind: "activate" }));
  const inputPointer = exports.tine_alloc(event.length);
  assert.notEqual(inputPointer, 0, "fixture allocator rejected a bounded event");
  new Uint8Array(memory.buffer).set(event, inputPointer);
  const resultPointer = exports.tine_handle(inputPointer, event.length);
  const resultLength = exports.tine_result_len();
  const response = Buffer.from(memory.buffer, resultPointer, resultLength).toString("utf8");
  assert.deepEqual(JSON.parse(response), { protocolVersion: 2, effects: [] });
  assert.equal(exports.tine_alloc(256 * 1024 + 1), 0, "fixture allocator exceeded the SDK input limit");
}

try {
  const first = buildAt("absolute-root-a");
  const second = buildAt("different-absolute-root-b");
  assert.equal(sha256(first), sha256(second), "locked fixture bytes vary across absolute source roots");
  assert.deepEqual(first, fs.readFileSync(committedWasm), "committed fixture differs from a clean locked build");
  assertRunnableAbi(first);
  console.log(`PASS: two-root revocation fixture is byte-identical and runnable (${sha256(first)})`);
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
