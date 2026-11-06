import { beforeEach, vi } from "vitest";

// Override only the switch module, leaving the production/test-off policy alone.
vi.mock("../editor/cm6Setting", async importOriginal => ({
  ...await importOriginal<typeof import("../editor/cm6Setting")>(),
  cm6EditorEnabled: () => true,
}));

// Dual-engine files (useEditorKind) already run their CM6 half in the ordinary
// render suite; their textarea half cannot run here, where CM6 is forced on.
(globalThis as { __tineCm6Suite?: boolean }).__tineCm6Suite = true;

beforeEach(() => {
  // jsdom has no layout. Supply empty geometry, never fabricated visual rows.
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
