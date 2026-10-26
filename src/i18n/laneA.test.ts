import { describe, expect, it } from "vitest";
import { BUILTIN_KEYS } from "../keybindings";
import { KNOBS } from "../styleKnobs";
import { SETTING_SEARCH } from "../components/settingsSearch";

describe("localized interface tables (GH #107)", () => {
  it("keeps built-in shortcut prose deferred until a reactive consumer renders it", () => {
    for (const key of BUILTIN_KEYS) {
      expect(key.label).toHaveProperty("kind", "msg");
      if (key.details) expect(key.details).toHaveProperty("kind", "msg");
    }
  });
  it("keeps style and settings search labels in the catalog", () => {
    for (const knob of KNOBS) {
      expect(knob.title).toHaveProperty("kind", "msg");
      if (knob.description) expect(knob.description).toHaveProperty("kind", "msg");
    }
    for (const entry of SETTING_SEARCH) {
      expect(entry.label).toHaveProperty("kind", "msg");
      expect(entry.description).toHaveProperty("kind", "msg");
    }
  });
});
