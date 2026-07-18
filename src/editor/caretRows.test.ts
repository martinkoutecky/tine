// @vitest-environment jsdom

import { afterEach, describe, expect, it, vi } from "vitest";
import { caretAtFirstRow, caretAtLastRow } from "./caretRows";

afterEach(() => vi.restoreAllMocks());

describe("certain textarea row boundaries", () => {
  it("does not consult browser layout at the absolute start and end", () => {
    const textarea = document.createElement("textarea");
    textarea.value = "a long value that may wrap differently across browser hosts";
    const append = vi.spyOn(document.body, "appendChild");

    expect(caretAtFirstRow(textarea, 0)).toBe(true);
    expect(caretAtLastRow(textarea, textarea.value.length)).toBe(true);
    expect(append).not.toHaveBeenCalled();
  });
});
