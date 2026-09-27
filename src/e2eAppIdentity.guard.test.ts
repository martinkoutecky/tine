import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { APP_ID } from "../scripts/lib/app-identity.mjs";

const ROOT = path.resolve(__dirname, "..");
const SCRIPTS = path.join(ROOT, "scripts");

function scriptFiles(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return entry.name === "fixtures" ? [] : scriptFiles(full);
    return /\.(mjs|js|cjs)$/.test(entry.name) ? [full] : [];
  });
}

describe("E2E app identity", () => {
  it("reads the identifier the Rust app uses", () => {
    const rust = fs.readFileSync(path.join(ROOT, "src-tauri/src/app_identity.rs"), "utf8");
    expect(rust).toContain(`APP_IDENTIFIER: &str = "${APP_ID}"`);
  });

  it("no script spells an app identifier; use scripts/lib/app-identity.mjs", () => {
    // og runs as page.tine.TineOG beside master's page.tine.Tine. A journey that spells
    // either seeds settings the app never reads (the reference-completion journey
    // failed this way). Exemplar: scripts/e2e-og-parity-references.mjs.
    const offenders = scriptFiles(SCRIPTS)
      .filter((file) => !file.endsWith(path.join("lib", "app-identity.mjs")))
      .filter((file) => /page\.tine\.Tine/.test(fs.readFileSync(file, "utf8")))
      .map((file) => path.relative(ROOT, file));
    expect(offenders).toEqual([]);
  });
});
