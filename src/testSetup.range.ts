import { beforeEach } from "vitest";

beforeEach(() => {
  // jsdom has no layout. Supply empty geometry, never fabricated visual rows.
  if (!Range.prototype.getClientRects) Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  if (!Range.prototype.getBoundingClientRect) Range.prototype.getBoundingClientRect = () => new DOMRect();
});
