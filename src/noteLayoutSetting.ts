import { msg } from "./i18n";
// Note layout (GH #823): mirror the outline in note panes so bullets, indentation
// and guide lines sit on the right. Device-wide, like the spell checker's switch,
// in the existing app settings file; nothing goes into the graph. Until the user
// chooses, it follows the interface language's direction. A block's own text
// direction is automatic (render/textDirection.ts).
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { activeLocaleDirection } from "./i18n";
import { loadPreference, writePreference } from "./preferenceWrites";

const KEY = "note_layout";
type Choice = "" | "ltr" | "rtl";
const [choice, setChoice] = createSignal<Choice>("");

/** Reactive: are note panes mirrored? O(1); read once per mounted row. */
export const noteLayoutRtl = (): boolean => (choice() || activeLocaleDirection()) === "rtl";

/** The `dir` of an outline row. Absent while it matches the root, so a left-to-right
 * interface with the default layout has no attribute; the opposite of a right-to-left
 * interface names itself. */
export const noteLayoutDirAttr = (): "ltr" | "rtl" | undefined =>
  noteLayoutRtl() ? "rtl" : activeLocaleDirection() === "rtl" ? "ltr" : undefined;

/** Apply at once and queue the device write; a failed write rolls back and reports. */
export function setNoteLayoutRtl(on: boolean): void {
  writePreference(choice, setChoice, on ? "rtl" : "ltr", next => backend().setAppString(KEY, next), msg("toast-save-note-layout-failed"));
}

export function initNoteLayoutSetting(): void {
  loadPreference(choice, setChoice, () => backend().getAppString(KEY, ""),
    value => (value === "rtl" || value === "ltr" ? value : ""), "note layout");
}
