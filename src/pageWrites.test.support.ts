// Test-only: observe every page write command the window can issue to the page
// host (`page_submit`, `page_move`, `page_delete`). Not a test file itself.

import { expect, vi } from "vitest";
import { backend } from "./backend";

export function spyPageWrites() {
  const b = backend();
  return [vi.spyOn(b, "pageSubmit"), vi.spyOn(b, "pageMove"), vi.spyOn(b, "pageDelete")] as const;
}

export function expectNoPageWrites(writes: ReturnType<typeof spyPageWrites>): void {
  for (const write of writes) expect(write, `${write.getMockName()} wrote a page`).not.toHaveBeenCalled();
}
