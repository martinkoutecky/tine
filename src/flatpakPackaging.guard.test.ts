import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, expect, it } from "vitest";
import { IDENTITIES, SHIP } from "../scripts/lib/app-identity.mjs";
import { deriveFlatpak, resolveReleaseDate } from "../scripts/derive-flatpak-identity.mjs";

// The Flatpak recipe has ONE source of truth (flatpak/<released id>.yml, .desktop,
// .metainfo.xml); every other identity is derived from it at build time by
// scripts/derive-flatpak-identity.mjs (docs/app-identity.md). Exemplar for the
// scratch-tree pattern: src/appIdentity.guard.test.ts.
const ROOT = path.resolve(__dirname, "..");
const RELEASED = IDENTITIES.release.identifier as string;
const read = (file: string) => fs.readFileSync(path.join(ROOT, file), "utf8");

/** A scratch repo that ships `ship`, holding only what the derivation reads. */
function scratch(ship: string): string {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "tine-flatpak-derive-"));
  fs.mkdirSync(path.join(root, "src-tauri"), { recursive: true });
  fs.mkdirSync(path.join(root, "flatpak"), { recursive: true });
  const sw = JSON.parse(read("src-tauri/app-identity.json"));
  fs.writeFileSync(path.join(root, "src-tauri/app-identity.json"), JSON.stringify({ ...sw, ship }));
  for (const ext of [".yml", ".desktop", ".metainfo.xml"]) {
    fs.copyFileSync(path.join(ROOT, "flatpak", `${RELEASED}${ext}`), path.join(root, "flatpak", `${RELEASED}${ext}`));
  }
  return root;
}

describe("Flatpak identity derivation", () => {
  it("the released identity derives exactly the manifest the workflow always built", () => {
    const root = scratch("release");
    try {
      const derived = deriveFlatpak({ root, identity: "release", date: "2026-01-01", version: "1.2.3" });
      const stable = read(`flatpak/${RELEASED}.yml`);
      const expected =
        stable.slice(0, stable.indexOf("    sources:")) +
        "    sources:\n      - type: dir\n        path: ..\n      - cargo-sources.json\n      - node-sources.json\n";
      expect(Object.keys(derived.files)).toEqual([`flatpak/${RELEASED}.ci.yml`]);
      expect(derived.files[`flatpak/${RELEASED}.ci.yml`]).toBe(expected);
      expect(derived.id).toBe(RELEASED);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it("another identity gets its own app id, name and files, and nothing of the stable identity", () => {
    const beta = IDENTITIES.experiment as { identifier: string; productName: string };
    const root = scratch("experiment");
    try {
      const derived = deriveFlatpak({ root, identity: "experiment", date: "2026-01-01", version: "1.2.3-beta.4" });
      expect(derived.id).toBe(beta.identifier);
      expect(Object.keys(derived.files).sort()).toEqual([
        `flatpak/${beta.identifier}.ci.yml`,
        `flatpak/derived/${beta.identifier}.desktop`,
        `flatpak/derived/${beta.identifier}.metainfo.xml`,
      ].sort());
      const manifest = derived.files[`flatpak/${beta.identifier}.ci.yml`];
      const desktop = derived.files[`flatpak/derived/${beta.identifier}.desktop`];
      const meta = derived.files[`flatpak/derived/${beta.identifier}.metainfo.xml`];
      expect(manifest).toMatch(new RegExp(`^id: ${beta.identifier.replace(/\./g, "\\.")}$`, "m"));
      expect(desktop).toContain(`Name=${beta.productName}\n`);
      expect(desktop).toContain(`StartupWMClass=${beta.identifier}\n`);
      expect(desktop).toContain(`Icon=${beta.identifier}\n`);
      expect(meta).toContain(`<id>${beta.identifier}</id>`);
      expect(meta).toContain(`<launchable type="desktop-id">${beta.identifier}.desktop</launchable>`);
      expect(meta).toContain(`<name>${beta.productName}</name>`);
      expect(meta).toContain('<release version="1.2.3-beta.4" date="2026-01-01"/>');
      // No stable-identity token survives anywhere (the tokens are word-bounded:
      // the Beta id legitimately begins with the stable id's characters).
      const token = new RegExp(`${RELEASED.replace(/\./g, "\\.")}(?!\\w)`);
      for (const [file, text] of Object.entries(derived.files)) expect(text, file).not.toMatch(token);
      // Everything the manifest installs from the checkout is either derived here
      // or a file of the repository.
      for (const m of manifest.matchAll(/install -Dm\d+ ((?:flatpak|src-tauri)\/\S+)/g)) {
        expect(derived.files[m[1]] !== undefined || fs.existsSync(path.join(ROOT, m[1])), m[1]).toBe(true);
      }
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
  });

  it("refuses an identity the tree does not ship", () => {
    const root = scratch("experiment");
    try {
      expect(() => deriveFlatpak({ root, identity: "release", date: "2026-01-01", version: "1.0.0" }))
        .toThrow(/ships the experiment identity/);
      expect(() => deriveFlatpak({ root, identity: "nonsense", date: "2026-01-01", version: "1.0.0" }))
        .toThrow(/unknown identity/);
    } finally {
      fs.rmSync(root, { recursive: true, force: true });
    }
    const released = scratch("release");
    try {
      expect(() => deriveFlatpak({ root: released, identity: "experiment", date: "2026-01-01", version: "1.0.0" }))
        .toThrow(/ships the release identity/);
    } finally {
      fs.rmSync(released, { recursive: true, force: true });
    }
  });

  it("the derived metainfo of the identity this tree ships is accepted by the derivation", () => {
    // Runs the real tree end to end (not a scratch copy): fails if a stable file
    // loses a shape the derivation relies on.
    const conf = JSON.parse(read("src-tauri/tauri.conf.json"));
    expect(() => deriveFlatpak({ identity: SHIP, date: "2026-01-01", version: conf.version })).not.toThrow();
  });
});

describe("Flatpak workflows", () => {
  it("flatpak.yml derives through the identity-aware script and takes the identity as input", () => {
    const wf = read(".github/workflows/flatpak.yml");
    expect(wf).toMatch(/workflow_call:\n {4}inputs:\n {6}identity:/);
    expect(wf).toMatch(/workflow_dispatch:\n {4}inputs:\n {6}identity:/);
    expect(wf).toContain('node scripts/derive-flatpak-identity.mjs --identity "$FLATPAK_IDENTITY"');
    expect(wf).toContain("manifest-path: ${{ steps.derive.outputs.manifest }}");
  });

  it("release.yml packages the identity this tree ships", () => {
    // Flipping src-tauri/app-identity.json makes the derivation refuse a stale
    // value; this fails first, naming the line to change.
    const wf = read(".github/workflows/release.yml");
    const named = /\n {2}flatpak:\n {4}needs: preflight\n {4}uses: \.\/\.github\/workflows\/flatpak\.yml\n {4}with:\n {6}identity: (\w+)\n/.exec(wf)?.[1];
    expect(named, "release.yml must call flatpak.yml with an identity").toBe(SHIP);
  });
});

describe("Flatpak tray library module", () => {
  const vendored = "flatpak/shared-modules/libayatana-appindicator/libayatana-appindicator-gtk3.json";

  it("the manifest builds libayatana-appindicator before Tine, from the vendored shared module", () => {
    const manifest = read(`flatpak/${RELEASED}.yml`);
    const module = manifest.indexOf(`  - shared-modules/libayatana-appindicator/libayatana-appindicator-gtk3.json`);
    expect(module).toBeGreaterThan(manifest.indexOf("\nmodules:"));
    expect(module).toBeLessThan(manifest.indexOf("  - name: tine"));
    // The other half of the tray permission set (tray.rs loads the library by soname).
    expect(manifest).toContain("--talk-name=org.kde.StatusNotifierWatcher");
    expect(read("src-tauri/src/tray.rs")).toContain("libayatana-appindicator3.so.1");
  });

  it("every vendored source is pinned and every patch it applies is present", () => {
    const seen: string[] = [];
    const walk = (module: any, dir: string) => {
      for (const sub of module.modules ?? []) {
        if (typeof sub === "string") {
          const file = path.join(dir, sub);
          walk(JSON.parse(fs.readFileSync(path.join(ROOT, file), "utf8")), path.dirname(file));
        } else walk(sub, dir);
      }
      for (const source of module.sources ?? []) {
        seen.push(`${module.name}:${source.type}`);
        if (source.type === "git") expect(source.commit, `${module.name} git source`).toMatch(/^[0-9a-f]{40}$/);
        else if (source.type === "archive") expect(source.sha256, `${module.name} archive`).toMatch(/^[0-9a-f]{64}$/);
        else if (source.type === "patch") {
          for (const patch of source.paths ?? [source.path]) {
            expect(fs.existsSync(path.join(ROOT, dir, patch)), `${module.name} patch ${patch}`).toBe(true);
          }
        } else throw new Error(`unreviewed source type ${source.type} in ${module.name}`);
      }
    };
    walk(JSON.parse(read(vendored)), path.dirname(vendored));
    expect(seen).toEqual(expect.arrayContaining([
      "libayatana-appindicator:git",
      "libdbusmenu:archive",
      "ayatana-ido:git",
      "libayatana-indicator:git",
      "intltool:archive",
    ]));
  });
});

describe("Flatpak metainfo release date", () => {
  it("is the source commit's date, never the build date, so one SHA derives the same files", () => {
    // 2026-10-06T23:59:59Z: a build run the next day must still say 2026-10-06.
    const epoch = String(Date.UTC(2026, 9, 6, 23, 59, 59) / 1000);
    expect(resolveReleaseDate({ env: { SOURCE_DATE_EPOCH: epoch }, gitCommitDate: () => "1999-01-01" })).toBe("2026-10-06");
    expect(resolveReleaseDate({ env: {}, gitCommitDate: () => "2026-10-05" })).toBe("2026-10-05");
    expect(resolveReleaseDate({ explicit: "2026-01-02", env: { SOURCE_DATE_EPOCH: epoch } })).toBe("2026-01-02");
  });

  it("refuses to guess today's date when no commit date exists, and rejects malformed input", () => {
    expect(() => resolveReleaseDate({ env: {}, gitCommitDate: () => null })).toThrow(/no release date/);
    expect(() => resolveReleaseDate({ explicit: "yesterday" })).toThrow(/YYYY-MM-DD/);
    expect(() => resolveReleaseDate({ env: { SOURCE_DATE_EPOCH: "soon" } })).toThrow(/whole seconds/);
  });

  it("the CLI no longer reads the clock", () => {
    expect(read("scripts/derive-flatpak-identity.mjs")).not.toMatch(/new Date\(\)/);
  });
});

describe("Flatpak filesystem reach (the Guide says home folder only)", () => {
  it("the manifest grants exactly the home folder, so the Guide's limit is true", () => {
    const fs_args = read(`flatpak/${RELEASED}.yml`).match(/^\s*- --filesystem=\S+/gm)?.map((l) => l.trim());
    expect(fs_args).toEqual(["- --filesystem=home"]);
  });
});
