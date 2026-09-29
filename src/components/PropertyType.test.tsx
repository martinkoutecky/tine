import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { PropertyType } from "./PropertyType";
import { readPageProperty, resetStore } from "../document";
import { setDoc } from "../document/model";
import type { RegistryRow } from "../editor/queryIr";

afterEach(() => { resetStore(); document.body.replaceChildren(); });

describe("property type declaration", () => {
  it("labels observation and writes the declared type on the normalized key page", async () => {
    setDoc({ byId: {}, pages: [{ name: "due-date", kind: "page", title: "due-date", preBlock: null,
      roots: [], format: "md", readOnly: false, guide: false }], feed: [], loaded: true });
    const row: RegistryRow = { normalized_name: "due-date", observed_type: "text", cardinality: "one",
      count_blocks: 3, count_pages: 0, mismatch_count: 1 };
    const refresh = vi.fn();
    const host = document.createElement("div");
    document.body.appendChild(host);
    const dispose = render(() => <PropertyType propertyKey="due date" rows={() => [row]}
      onDeclarationWritten={refresh} />, host);
    try {
      expect(host.textContent).toContain("text (observed)");
      host.querySelector<HTMLButtonElement>("button")!.click();
      const date = [...host.querySelectorAll<HTMLButtonElement>("button")]
        .find((button) => button.textContent === "date");
      date!.click();
      await vi.waitFor(() => expect(readPageProperty("due-date", "tine.type")).toBe("date"));
      expect(refresh).toHaveBeenCalledOnce();
    } finally { dispose(); }
  });
});
