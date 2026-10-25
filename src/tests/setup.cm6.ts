import { beforeEach, vi } from "vitest";

// Override only the switch module, leaving the production/test-off policy alone.
vi.mock("../editor/cm6Spike", () => ({ cm6SpikeEnabled: () => true }));

beforeEach(() => {
  // jsdom has no layout. Supply empty geometry, never fabricated visual rows.
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
