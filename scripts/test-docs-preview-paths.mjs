#!/usr/bin/env node

import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { pathToFileURL } from "node:url";
import {
  resolveLaunchPluginRoots,
  resolveLaunchThemeRoots,
  resolveRepositoryRoot,
  resolveViteExecutable,
} from "./docs-preview-paths.mjs";

const MARKER = "checkout-local-vite-sentinel";
const tempRoot = await mkdtemp(join(tmpdir(), "tine-docs-preview-paths-"));

function runSentinel(executable) {
  return new Promise((resolve, reject) => {
    const child = spawn(executable, [], {
      encoding: "utf8",
      shell: process.platform === "win32",
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => { stdout += chunk; });
    child.stderr.on("data", (chunk) => { stderr += chunk; });
    child.once("error", reject);
    child.once("close", (code) => {
      if (code !== 0) {
        reject(new Error(`Vite sentinel exited ${code}: ${stderr}`));
        return;
      }
      resolve(stdout.trim());
    });
  });
}

try {
  const checkout = join(tempRoot, "standalone-tine");
  const scripts = join(checkout, "scripts");
  const syntheticModuleUrl = pathToFileURL(join(scripts, "shot-plugin-docs.mjs"));
  const repoRoot = resolveRepositoryRoot(syntheticModuleUrl);
  assert.equal(repoRoot, checkout);

  const vite = resolveViteExecutable(repoRoot);
  assert.equal(
    resolveViteExecutable(repoRoot, "win32"),
    join(checkout, "node_modules", ".bin", "vite.cmd"),
  );
  const pluginRoots = resolveLaunchPluginRoots(repoRoot, {});
  const expectedPlugins = {
    bullet: join(checkout, "community-plugins", "bullet-threading"),
    query: join(checkout, "community-plugins", "query-filter"),
    heading: join(checkout, "community-plugins", "heading-level-shortcuts"),
  };

  await mkdir(dirname(vite), { recursive: true });
  await Promise.all(Object.values(expectedPlugins).map((directory) => mkdir(directory, { recursive: true })));
  if (process.platform === "win32") {
    await writeFile(vite, `@echo off\r\necho ${MARKER}\r\n`, "utf8");
  } else {
    await writeFile(vite, `#!/bin/sh\nprintf '%s\\n' '${MARKER}'\n`, "utf8");
    await chmod(vite, 0o755);
  }

  assert.deepEqual(pluginRoots, expectedPlugins);
  for (const resolved of [vite, ...Object.values(pluginRoots)]) {
    assert.equal(resolved.startsWith(`${checkout}/`) || resolved.startsWith(`${checkout}\\`), true);
  }
  assert.equal(await runSentinel(vite), MARKER);

  assert.deepEqual(resolveLaunchThemeRoots(repoRoot, {}), {
    dev: join(tempRoot, "tine-theme-dev"),
    things: join(tempRoot, "tine-theme-things"),
  });

  const overrides = {
    TINE_BULLET_PLUGIN_ROOT: "relative/bullet override",
    TINE_QUERY_PLUGIN_ROOT: "/explicit/query override",
    TINE_HEADING_PLUGIN_ROOT: "heading override",
  };
  assert.deepEqual(resolveLaunchPluginRoots(repoRoot, overrides), {
    bullet: overrides.TINE_BULLET_PLUGIN_ROOT,
    query: overrides.TINE_QUERY_PLUGIN_ROOT,
    heading: overrides.TINE_HEADING_PLUGIN_ROOT,
  });

  const themeOverrides = {
    TINE_DEV_THEME_ROOT: "dev override",
    TINE_THINGS_THEME_ROOT: "things override",
  };
  assert.deepEqual(resolveLaunchThemeRoots(repoRoot, themeOverrides), {
    dev: themeOverrides.TINE_DEV_THEME_ROOT,
    things: themeOverrides.TINE_THINGS_THEME_ROOT,
  });

  console.log("Docs preview paths stay checkout-local and launch the checkout-local Vite executable.");
} finally {
  await rm(tempRoot, { recursive: true, force: true });
}
