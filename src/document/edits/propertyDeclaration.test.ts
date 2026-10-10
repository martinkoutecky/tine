import { afterEach, describe, expect, it, vi } from "vitest";
import { ensurePagePropertyOnKeyPage, flushPage, pageByName, readPageProperty, resetStore } from "..";
import { setDoc } from "../model";
import { loadSingle } from "../workingSet";
import { backend } from "../../backend";
import { bindTestHost, submittedPages } from "../host/wiring.test.support";

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

  it("reopens the declared type from the one saved page payload", async () => {
    resetStore();
    setDoc({ byId: {}, pages: [], feed: [], loaded: true });
    await bindTestHost();
    vi.spyOn(backend(), "getPage").mockResolvedValue(null);
    const save = vi.spyOn(backend(), "pageSubmit");
    await ensurePagePropertyOnKeyPage("cost", "tine.type", "number");
    expect(await flushPage("cost")).toBe(true);
    expect(save).toHaveBeenCalledTimes(1);
    const [payload] = submittedPages(save, "cost");
    expect(payload.pre_block).toContain("tine.type:: number");
    resetStore();
    loadSingle(payload);
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
