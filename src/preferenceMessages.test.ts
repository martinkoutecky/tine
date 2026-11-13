import { expectTypeOf, it } from "vitest";
import type { Msg } from "./i18n";
import { changeGraphSetting, writeGraphSignal } from "./graphPreferences";
import { writePreference } from "./preferenceWrites";
import type { GraphMeta } from "./types";

it("requires unresolved catalog messages at every preference writer", () => {
  expectTypeOf<Parameters<typeof writePreference>[4]>().toEqualTypeOf<Msg>();
  expectTypeOf<Parameters<typeof writeGraphSignal>[5]>().toEqualTypeOf<Msg>();
  expectTypeOf<Parameters<typeof changeGraphSetting>[3]>().toEqualTypeOf<Msg>();
});

it("receives effective config values from Rust rather than optional frontend defaults", () => {
  type BooleanSetting = "doc_mode_enter_for_new_block" | "logical_outdenting" |
    "enable_search_remove_accents" | "quick_capture_insert_today" | "quick_capture_redirect_page";
  expectTypeOf<GraphMeta[BooleanSetting]>().toEqualTypeOf<boolean>();
  expectTypeOf<GraphMeta["file_name_format"]>().toEqualTypeOf<"legacy" | "triple-lowbar">();
  type NullableSetting = "quick_capture_template_text" | "quick_capture_template_media" | "quick_capture_default_page";
  expectTypeOf<GraphMeta[NullableSetting]>().toEqualTypeOf<string | null>();
});
