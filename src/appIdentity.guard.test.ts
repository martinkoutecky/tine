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
// The released identity's packaging: Flatpak names its files and ids by it; its
// workflow refuses an experiment tree (.github/workflows/flatpak.yml).
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
        for (const key of ["identifier", "androidApplicationId", "deployName"]) {
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
    expect(fs.readFileSync(path.join(ROOT, ".github/workflows/flatpak.yml"), "utf8")).toContain('["ship"] != "release"');
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
