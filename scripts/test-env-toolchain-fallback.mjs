#!/usr/bin/env node
import assert from "node:assert/strict";
import { chmodSync, copyFileSync, mkdtempSync, mkdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = path.resolve(fileURLToPath(new URL(".", import.meta.url)), "..");
const fixture = mkdtempSync(path.join(tmpdir(), "tine-env-toolchain-"));
const checkout = path.join(fixture, "checkout");
const fixtureEnv = path.join(checkout, "scripts", "env.sh");
const fakeBin = path.join(fixture, "bin");
const callerPath = `${fakeBin}:${process.env.PATH}`;
const callerEnvironment = {
  CARGO_HOME: "/caller/cargo",
  RUSTUP_HOME: "/caller/rustup",
  PATH: callerPath,
  PLAYWRIGHT_BROWSERS_PATH: "/caller/browsers",
  LD_LIBRARY_PATH: "/caller/lib",
  PKG_CONFIG_SYSROOT_DIR: "/caller/sysroot",
  PKG_CONFIG_PATH: "/caller/pkgconfig",
  CC: "/caller/missing-cc",
  CXX: "/caller/missing-cxx",
  AR: "/caller/missing-ar",
  RANLIB: "/caller/missing-ranlib",
  NM: "/caller/missing-nm",
  LD: "/caller/missing-ld",
  STRIP: "/caller/missing-strip",
  CFLAGS: "-I/caller/conda/sysroot",
  CXXFLAGS: "-I/caller/conda/sysroot",
  CPPFLAGS: "-I/caller/conda/sysroot",
  LDFLAGS: "-L/caller/conda/sysroot",
};
const toolOverrides = ["CC", "CXX", "AR", "RANLIB", "NM", "LD", "STRIP", "CFLAGS", "CXXFLAGS", "CPPFLAGS", "LDFLAGS"];

function readEnvironment({ toolchain, worktree }) {
  const env = { ...process.env, ...callerEnvironment, TINE_TEST_WORKTREE: worktree };
  delete env.TINE_TOOLCHAIN;
  if (toolchain !== undefined) env.TINE_TOOLCHAIN = toolchain;

  const result = spawnSync(
    "bash",
    ["-c", 'source "$1"\nprintf "TINE_TOOLCHAIN=%s\\0" "${TINE_TOOLCHAIN:-}"\nenv -0', "bash", fixtureEnv],
    { encoding: "buffer", env },
  );
  assert.equal(result.status, 0, result.stderr.toString());

  return new Map(
    result.stdout
      .toString()
      .split("\0")
      .filter(Boolean)
      .map((entry) => {
        const delimiter = entry.indexOf("=");
        return [entry.slice(0, delimiter), entry.slice(delimiter + 1)];
      }),
  );
}

function assertCallerEnvironment(environment) {
  for (const [name, value] of Object.entries(callerEnvironment)) {
    assert.equal(environment.get(name), value, `${name} must remain the caller value`);
  }
}

function assertToolchainEnvironment(environment, toolchain) {
  assert.equal(environment.get("TINE_TOOLCHAIN"), toolchain);
  assert.equal(environment.get("CARGO_HOME"), `${toolchain}/cargo`);
  assert.equal(environment.get("RUSTUP_HOME"), `${toolchain}/rustup`);
  assert.equal(environment.get("PATH"), `${toolchain}/cargo/bin:${callerPath}`);
  assert.equal(environment.get("PLAYWRIGHT_BROWSERS_PATH"), `${toolchain}/ms-playwright`);
  assert.equal(environment.get("LD_LIBRARY_PATH"), `${toolchain}/extralibs/root/usr/lib/x86_64-linux-gnu:/caller/lib`);
  assert.equal(environment.get("PKG_CONFIG_SYSROOT_DIR"), `${toolchain}/extralibs/root`);
  assert.equal(
    environment.get("PKG_CONFIG_PATH"),
    `${toolchain}/extralibs/root/usr/lib/x86_64-linux-gnu/pkgconfig:${toolchain}/extralibs/root/usr/share/pkgconfig:/caller/pkgconfig`,
  );
  for (const name of toolOverrides) {
    assert.equal(environment.get(name), undefined, `${name} must retain local-toolchain cleanup behavior`);
  }
}

try {
  mkdirSync(path.dirname(fixtureEnv), { recursive: true });
  copyFileSync(path.join(root, "scripts", "env.sh"), fixtureEnv);
  mkdirSync(fakeBin);
  writeFileSync(
    path.join(fakeBin, "git"),
    "#!/bin/sh\nif [ -n \"${TINE_TEST_WORKTREE:-}\" ]; then\n  printf 'worktree %s\\n' \"$TINE_TEST_WORKTREE\"\nfi\n",
  );
  chmodSync(path.join(fakeBin, "git"), 0o755);

  const fallback = readEnvironment({ worktree: "" });
  assert.equal(fallback.get("TINE_TOOLCHAIN"), "");
  assertCallerEnvironment(fallback);

  const discoveredRoot = path.join(fixture, "persistent", ".toolchain");
  const discoveredWorktree = path.join(fixture, "persistent", "worktree");
  mkdirSync(path.join(discoveredRoot, "cargo", "bin"), { recursive: true });
  mkdirSync(discoveredWorktree, { recursive: true });
  writeFileSync(path.join(discoveredRoot, "cargo", "bin", "rustup"), "#!/bin/sh\nexit 0\n");
  chmodSync(path.join(discoveredRoot, "cargo", "bin", "rustup"), 0o755);
  assertToolchainEnvironment(readEnvironment({ worktree: discoveredWorktree }), discoveredRoot);

  const explicitRoot = path.join(fixture, "explicit-without-rustup");
  assertToolchainEnvironment(readEnvironment({ toolchain: explicitRoot, worktree: "" }), explicitRoot);
} finally {
  rmSync(fixture, { force: true, recursive: true });
}

console.log("env toolchain fallback contract: ok");
