import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { APP_ID, IDENTITIES, SHIP, deriveIdentityFiles } from "../scripts/lib/app-identity.mjs";

// The app identity has ONE switch: src-tauri/app-identity.json (docs/app-identity.md).
// Every identity-bearing file derives from it; nothing else spells an identity.
// Exemplar for scripts: scripts/e2e-og-parity-references.mjs (imports APP_ID).
const ROOT = path.resolve(__dirname, "..");

function files(dir: string, keep: RegExp): string[] {
  if (!fs.existsSync(dir)) return [];
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      return ["node_modules", "target", "build", ".gradle", "fixtures"].includes(entry.name) ? [] : files(full, keep);
    }
    return keep.test(entry.name) ? [full] : [];
  });
}

const DERIVED = ["src-tauri/app-identity.json", "src-tauri/tauri.conf.json", "src-tauri/gen/android/app/build.gradle.kts"];
// The released identity's packaging: Flatpak names its files and ids by it; other
// identities are derived from those files at build time
// (scripts/derive-flatpak-identity.mjs, .github/workflows/flatpak.yml).
const RELEASE_PACKAGING = /^flatpak\//;
const FRONT_DOOR = "scripts/lib/app-identity.mjs";

function identityLiterals(): RegExp {
  // `page.tine.app` is also the Kotlin package of the Android sources, so the Android
  // application ids are enforced through the derived gradle file instead.
  const values = Object.values(IDENTITIES as Record<string, { identifier: string; productName: string }>)
    .flatMap((identity) => [identity.identifier, identity.productName])
    .filter((value) => value !== "Tine");
  return new RegExp(`(${values.map((v) => v.replace(/\./g, "\\.")).join("|")})(?![\\w.])`);
}

describe("app identity switch", () => {
  it("ships the Beta identity while retaining the stable seed source and local deploy name", () => {
    expect(IDENTITIES.experiment).toEqual({ identifier: "page.tine.TineBeta", productName: "Tine Beta", androidApplicationId: "page.tine.beta", deployName: "tine-og" });
    expect(SHIP).toBe("experiment");
  });
  it("every derived file matches the switch (run `node scripts/set-app-identity.mjs <ship>`)", () => {
    for (const [file, text] of Object.entries(deriveIdentityFiles(ROOT, SHIP))) {
      expect(fs.readFileSync(path.join(ROOT, file), "utf8"), file).toBe(text);
    }
    const conf = JSON.parse(fs.readFileSync(path.join(ROOT, "src-tauri/tauri.conf.json"), "utf8"));
    expect(conf.identifier).toBe(APP_ID);
  });

  it("both settings of the switch derive a complete, distinct identity and round-trip", () => {
    const scratch = fs.mkdtempSync(path.join(os.tmpdir(), "tine-identity-"));
    try {
      for (const file of DERIVED) {
        fs.mkdirSync(path.dirname(path.join(scratch, file)), { recursive: true });
        fs.copyFileSync(path.join(ROOT, file), path.join(scratch, file));
      }
      const seen = new Set<string>();
      for (const ship of Object.keys(IDENTITIES)) {
        const identity = IDENTITIES[ship];
        const derived = deriveIdentityFiles(scratch, ship);
        for (const [file, text] of Object.entries(derived)) fs.writeFileSync(path.join(scratch, file), text as string);
        const conf = JSON.parse(derived["src-tauri/tauri.conf.json"]);
        expect(conf.identifier).toBe(identity.identifier);
        expect(conf.productName).toBe(identity.productName);
        expect(conf.app.windows.find((w: { label: string }) => w.label === "main").title).toBe(identity.productName);
        expect(derived["src-tauri/gen/android/app/build.gradle.kts"]).toContain(`applicationId = "${identity.androidApplicationId}"`);
        expect(JSON.parse(derived["src-tauri/app-identity.json"]).ship).toBe(ship);
        for (const key of ["identifier", "androidApplicationId", "deployName"] as const) {
          expect(seen.has(`${key}=${identity[key]}`), `${ship} shares ${key}`).toBe(false);
          seen.add(`${key}=${identity[key]}`);
        }
      }
      // Flipping back to the current setting reproduces the checked-in bytes.
      for (const [file, text] of Object.entries(deriveIdentityFiles(scratch, SHIP))) {
        expect(text, file).toBe(fs.readFileSync(path.join(ROOT, file), "utf8"));
      }
    } finally {
      fs.rmSync(scratch, { recursive: true, force: true });
    }
  });

  it("the released identity is master's, and Flatpak packaging carries it", () => {
    expect(IDENTITIES.release.identifier).toBe("page.tine.Tine");
    expect(IDENTITIES.release.androidApplicationId).toBe("page.tine.app");
    const manifest = fs.readFileSync(path.join(ROOT, "flatpak/page.tine.Tine.yml"), "utf8");
    expect(manifest).toMatch(new RegExp(`^id: ${IDENTITIES.release.identifier.replace(/\./g, "\\.")}$`, "m"));
    // The workflow no longer hard-codes the released identity: it passes its `identity`
    // input to scripts/derive-flatpak-identity.mjs, which refuses any identity the
    // tree does not ship (covered in src/flatpakPackaging.guard.test.ts).
    expect(fs.readFileSync(path.join(ROOT, ".github/workflows/flatpak.yml"), "utf8")).toContain("derive-flatpak-identity.mjs --identity");
  });

  it("the experiment config seed exists only while the build ships an experiment identity", () => {
    // Scope D is a temporary convenience (docs/app-identity.md): when the switch
    // ships the released identity, delete src-tauri/src/experiment_config_seed.rs
    // (+ its tests file, `mod` line and call in lib::run, and its writer-site
    // approval in scripts/lib/og-enforcement.mjs).
    const seed = fs.existsSync(path.join(ROOT, "src-tauri/src/experiment_config_seed.rs"));
    expect(seed).toBe(SHIP !== "release");
  });

  it("docs/app-identity.md states the switch and the seed allowlist the code uses", () => {
    const doc = fs.readFileSync(path.join(ROOT, "docs/app-identity.md"), "utf8");
    for (const identity of Object.values(IDENTITIES)) {
      for (const value of Object.values(identity)) expect(doc, String(value)).toContain(`"${value}"`);
    }
    const seedPath = path.join(ROOT, "src-tauri/src/experiment_config_seed.rs");
    if (!fs.existsSync(seedPath)) return;
    const block = /const CONFIG_ENTRIES: &\[&str\] = &\[([\s\S]*?)\];/.exec(fs.readFileSync(seedPath, "utf8"));
    const entries = [...(block?.[1] ?? "").matchAll(/"([^"]+)"/g)].map((match) => match[1]);
    expect(entries.length).toBeGreaterThan(0);
    const documented = /allowlist \(([^)]*)\)/.exec(doc.replace(/\s+/g, " "))?.[1] ?? "";
    expect([...documented.matchAll(/`([^`]+)`/g)].map((match) => match[1].replace(/\/$/, ""))
      .filter((entry) => !entry.startsWith("."))).toEqual(entries);
  });

  it("no source outside the switch and its derived files spells an identity", () => {
    const pattern = identityLiterals();
    const scanned = [
      ...files(path.join(ROOT, "src"), /\.(ts|tsx|js|mjs)$/),
      ...files(path.join(ROOT, "scripts"), /\.(mjs|js|cjs|sh|rs)$/),
      ...files(path.join(ROOT, "src-tauri"), /\.(rs|json|kts|kt|xml|plist)$/),
      ...files(path.join(ROOT, "crates"), /\.rs$/),
    ];
    const offenders = scanned
      .map((file) => path.relative(ROOT, file))
      .filter((file) => !DERIVED.includes(file) && file !== FRONT_DOOR && !RELEASE_PACKAGING.test(file))
      .filter((file) => !file.startsWith("src-tauri/gen/schemas/") && !file.startsWith("src-tauri/schemas/"))
      .filter((file) => file !== "src/appIdentity.guard.test.ts")
      .filter((file) => pattern.test(stripComments(fs.readFileSync(path.join(ROOT, file), "utf8"))));
    expect(offenders).toEqual([]);
  });
});

/** Identity names in comments are prose, not identity; only code is scanned. */
function stripComments(text: string): string {
  return text.replace(/\/\*[\s\S]*?\*\//g, "").replace(/(^|[^:])\/\/.*$/gm, "$1").replace(/^\s*#.*$/gm, "");
}
