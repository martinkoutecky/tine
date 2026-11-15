import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { APP_ID, ROOT } from "./app-identity.mjs";
import { seedClassicEditorProfile } from "./e2e-classic-profile.mjs";

test("classic runner profile preserves explicit engines and unrelated device settings", () => {
  const out = path.join(ROOT, "target/artifacts/cm6-default-1114/profile-tests");
  fs.mkdirSync(out, { recursive: true });
  for (const platform of ["linux", "win32"]) {
    const data = fs.mkdtempSync(path.join(out, `${platform}-`));
    const env = { E2E_CLASSIC_PROFILE: "1", XDG_DATA_HOME: data, APPDATA: data };
    const file = path.join(data, APP_ID, "tine-settings.json");
    seedClassicEditorProfile(env, platform);
    assert.deepEqual(JSON.parse(fs.readFileSync(file, "utf8")), { experimental_cm6_editor: false });
    for (const choice of [true, false]) {
      const original = JSON.stringify({ experimental_cm6_editor: choice, unrelated: "kept" }, null, 2);
      fs.writeFileSync(file, original);
      seedClassicEditorProfile(env, platform);
      assert.equal(fs.readFileSync(file, "utf8"), original);
    }
    fs.writeFileSync(file, JSON.stringify({ unrelated: "kept" }));
    seedClassicEditorProfile(env, platform);
    assert.deepEqual(JSON.parse(fs.readFileSync(file, "utf8")), { unrelated: "kept", experimental_cm6_editor: false });
  }
});

test("fresh-default opt-out needs no profile and seeding never falls back to account storage", () => {
  seedClassicEditorProfile({ E2E_CLASSIC_PROFILE: "0" });
  assert.throws(() => seedClassicEditorProfile({ E2E_CLASSIC_PROFILE: "1" }), /absolute scratch/);
});

test("portable journey seeds its own settings without reading or writing host AppData", () => {
  const out = path.join(ROOT, "target/artifacts/cm6-default-1114/profile-tests");
  fs.mkdirSync(out, { recursive: true });
  const run = fs.mkdtempSync(path.join(out, "portable-"));
  const host = path.join(run, "host-appdata");
  const portable = path.join(run, "tine-data", APP_ID, "roaming");
  seedClassicEditorProfile({ E2E_CLASSIC_PROFILE: "1", APPDATA: host, E2E_CLASSIC_SETTINGS_DIR: portable }, "win32");
  assert.equal(fs.existsSync(host), false);
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(portable, "tine-settings.json"), "utf8")), { experimental_cm6_editor: false });
});
