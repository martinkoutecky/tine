import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("I-12: crossing notices use the graph record, never the device boolean; imitate settings.rs", () => {
  const native = readFileSync("src-tauri/src/settings.rs", "utf8");
  const get = native.split("pub(crate) fn get_app_bool(")[1].split("pub(crate) fn device_bool")[0];
  const set = native.split("pub(crate) fn set_app_bool(")[1].split("/// Generic device-local STRING")[0];
  expect(get).toContain('if key != "queryCrossingNoticeDismissed"');
  expect(get).toContain("slot_for_context(&state)");
  expect(set).toContain("set_crossing_notice_at(&dir, &slot.root_key, value)");
  expect(set).toContain("slot_for_context(&state)?");
  const macro = readFileSync("src/components/Macro.tsx", "utf8");
  expect(macro).toContain("crossingNoticePrimed === graphEpoch()");
  expect(macro).toContain("revisionOwner(crossingNoticePreference, revision, graphOwner())");
});
