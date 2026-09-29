import { afterEach, describe, expect, it, vi } from "vitest";
import { ensurePagePropertyOnKeyPage, pageByName, readPageProperty, resetStore } from "..";
import { setDoc } from "../model";
import { backend } from "../../backend";

afterEach(() => { vi.restoreAllMocks(); resetStore(); });

describe("property declaration write door", () => {
  it("uses the loaded normalized key page and its ordinary page edit path", async () => {
    setDoc({ byId: {}, pages: [{ name: "due-date", kind: "page", title: "due-date", preBlock: null,
      roots: [], format: "md", readOnly: false, guide: false }], feed: [], loaded: true });
    await ensurePagePropertyOnKeyPage("due-date", "tine.type", "date");
    expect(readPageProperty("due-date", "tine.type")).toBe("date");
  });

  it("creates an absent key page through the ordinary property edit", async () => {
    setDoc({ byId: {}, pages: [], feed: [], loaded: true });
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    await ensurePagePropertyOnKeyPage("cost", "tine.type", "number");
    expect(readPageProperty("cost", "tine.type")).toBe("number");
  });

  it("drops a page read that lands after a graph reset", async () => {
    setDoc({ byId: {}, pages: [], feed: [], loaded: true });
    let resolve!: (value: null) => void;
    vi.spyOn(backend(), "getPage").mockReturnValue(new Promise((done) => { resolve = done; }));
    const pending = ensurePagePropertyOnKeyPage("cost", "tine.type", "number");
    resetStore();
    resolve(null);
    await expect(pending).rejects.toThrow("graph changed");
    expect(pageByName("cost")).toBeUndefined();
  });
});
