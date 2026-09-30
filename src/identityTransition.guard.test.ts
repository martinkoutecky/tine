import fs from "node:fs";
import { describe, expect, it } from "vitest";

describe("OG-R6 startup data boundary", () => {
  it("release migration runs before settings and Tauri; experiment seed stays desktop-only", () => {
    const lib = fs.readFileSync("src-tauri/src/lib.rs", "utf8");
    expect(lib.indexOf("migrate_identifier::run_early();")).toBeGreaterThan(lib.indexOf("data_home::ensure_usable("));
    expect(lib.indexOf("migrate_identifier::run_early();")).toBeLessThan(lib.indexOf("settings::init_native_frame_active()"));
    expect(lib).toContain("migrate_identifier::take_identifier_migration_notice");
    const migration = fs.readFileSync("src-tauri/src/migrate_identifier.rs", "utf8");
    expect(migration).toContain("APP_IDENTIFIER != crate::app_identity::RELEASE_IDENTIFIER");
    expect(migration).toContain("publish_directory_entry");
    expect(migration.split("#[cfg(test)]")[0]).not.toMatch(/std::fs::(?:rename|copy|write)\(/);
  });
});
