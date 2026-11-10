import { afterEach, beforeEach } from "vitest";
import { setCm6EditorEnabled } from "../editor/cm6Setting";
import type { EditorKind } from "./editorDriver";
import { preloadCm6Editor } from "../components/EditorGate";

/** Run the enclosing `describe` on one editor engine. Call it inside
 * `describe.each<EditorKind>(["textarea", "cm6"])`. CM6 measures ranges, even
 * after a test ends; jsdom has no layout, so supply empty geometry (as
 * setup.cm6.ts does), never fabricated visual rows. Under vitest.cm6.config,
 * where CM6 is forced on, the textarea half is skipped: it would silently run
 * CM6, and the CM6 half already covers that engine. */
export function useEditorKind(kind: EditorKind): void {
  beforeEach(async ctx => {
    if (kind === "textarea" && (globalThis as { __tineCm6Suite?: boolean }).__tineCm6Suite) { ctx.skip(); return; }
    if (kind === "cm6") await preloadCm6Editor();
    if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
    if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
    setCm6EditorEnabled(kind === "cm6");
    await Promise.resolve();
  });
  afterEach(() => setCm6EditorEnabled(false));
}
