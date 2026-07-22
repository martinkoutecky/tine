#!/usr/bin/env node

import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sourceRoot = path.join(root, "fixtures/plugin-graph-ownership/sentinel-src");
const committedWasm = path.join(root, "fixtures/plugin-graph-ownership/plugin.wasm");
const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "tine-graph-owner-fixture-"));
const sha256 = (bytes) => crypto.createHash("sha256").update(bytes).digest("hex");
const MAX_MESSAGE_BYTES = 256 * 1024;

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
    "target/wasm32-unknown-unknown/release/tine_plugin_graph_owner_sentinel.wasm",
  ));
}

function assertRuntimeIdEcho(bytes) {
  const module = new WebAssembly.Module(bytes);
  assert.deepEqual(WebAssembly.Module.imports(module), [
    { module: "env", name: "memory", kind: "memory" },
  ]);
  const memory = new WebAssembly.Memory({ initial: 32, maximum: 256 });
  const { exports } = new WebAssembly.Instance(module, { env: { memory } });

  function invokeBytes(bytes) {
    const inputPointer = exports.tine_alloc(bytes.length);
    assert.notEqual(inputPointer, 0, "fixture allocator rejected a bounded event");
    new Uint8Array(memory.buffer).set(bytes, inputPointer);
    const outputPointer = exports.tine_handle(inputPointer, bytes.length);
    const outputLength = exports.tine_result_len();
    assert.ok(outputLength <= MAX_MESSAGE_BYTES, "fixture response exceeded the protocol limit");
    return JSON.parse(Buffer.from(memory.buffer, outputPointer, outputLength).toString("utf8"));
  }

  function invoke(event) {
    return invokeBytes(Buffer.from(JSON.stringify(event)));
  }

  for (const runtimeId of ["graph-a-runtime-31", "graph-b-runtime-72"]) {
    const response = invoke({
      protocolVersion: 2,
      kind: "command",
      contributionId: "delayed-write",
      focusedBlock: { id: runtimeId, raw: "same raw\nid:: shared-id", parentId: null, depth: 0, format: "md" },
    });
    assert.deepEqual(response, {
      protocolVersion: 2,
      effects: [{
        kind: "replace-block-text",
        blockId: runtimeId,
        expectedRaw: "same raw\nid:: shared-id",
        raw: "plugin result\nid:: shared-id",
      }],
    });
    assert.notEqual(response.effects[0].blockId, "shared-id", "authored ID escaped into the effect target");
  }

  assert.deepEqual(invoke({ protocolVersion: 2, kind: "command", contributionId: "delayed-write" }), {
    protocolVersion: 2,
    effects: [],
  });
  assert.deepEqual(invokeBytes(Buffer.from('{"protocolVersion":2,"kind":"command","focusedBlock":{"id":"unterminated}}')), {
    protocolVersion: 2,
    effects: [],
  });
  assert.equal(exports.tine_alloc(MAX_MESSAGE_BYTES + 1), 0, "fixture allocator exceeded the SDK input limit");
}

try {
  const first = buildAt("absolute-root-a");
  const second = buildAt("different-absolute-root-b");
  assert.equal(sha256(first), sha256(second), "locked fixture bytes vary across absolute source roots");
  assert.deepEqual(first, fs.readFileSync(committedWasm), "committed fixture differs from a clean locked build");
  assertRuntimeIdEcho(first);
  console.log(`PASS: graph-ownership fixture is reproducible and echoes runtime IDs (${sha256(first)})`);
} finally {
  fs.rmSync(temporary, { recursive: true, force: true });
}
