import { beforeEach, describe, expect, it, vi } from "vitest";
import type { PageEntry, PageInventory, PageInventoryEntry } from "./types";

const backendMock = vi.hoisted(() => ({
  pageInventory: vi.fn(),
}));

vi.mock("./backend", () => ({ backend: () => backendMock }));

const page = (name: string): PageEntry => ({
  name,
  kind: "page",
  date_key: null,
  path: `pages/${name.replaceAll("/", "___")}.md`,
});
const physical = (name: string): PageInventoryEntry => ({
  key: name.toLowerCase(),
  name,
  is_journal: false,
  day: null,
  target: { kind: "existing", id: page(name).path, others: [] },
});
const referenced = (name: string): PageInventoryEntry => ({
  key: name.toLowerCase(),
  name,
  is_journal: false,
  day: null,
  target: { kind: "absent", id: page(name).path },
});
const inventory = (rev: number, ...entries: PageInventoryEntry[]): PageInventory => ({ rev: String(rev), entries });

async function loadInventory() {
  const ui = await import("./ui");
  const pages = await import("./pages");
  return { ...ui, ...pages };
}

beforeEach(() => {
  vi.resetModules();
  backendMock.pageInventory.mockReset();
});

// All Pages and the complete name list are views of the one page index. The
// old test file pinned two IPCs (list_pages, referenced_page_names); each test
// keeps its outcome, now over the single page_inventory.
describe("GH #229 complete page-name inventory", () => {
  it("keeps All Pages physical-only while physical spelling wins its reference-only fold", async () => {
    backendMock.pageInventory.mockResolvedValue(inventory(1,
      physical("Test"), referenced("test"), referenced("test/testy test")));
    const { allPageNames, allPages } = await loadInventory();

    await vi.waitFor(() => {
      expect(allPages()).toEqual([page("Test")]);
      expect(allPageNames()).toEqual(["Test", "test/testy test"]);
    });
  });

  it("refreshes reference names after dataRev with one inventory IPC", async () => {
    let refs = ["test/first"];
    backendMock.pageInventory.mockImplementation(async () =>
      inventory(refs.length, physical("test"), ...refs.map(referenced)));
    const { allPageNames, bumpDataRev } = await loadInventory();

    await vi.waitFor(() => expect(allPageNames()).toEqual(["test", "test/first"]));
    backendMock.pageInventory.mockClear();
    refs = ["test/second"];
    bumpDataRev();

    await vi.waitFor(() => expect(allPageNames()).toEqual(["test", "test/second"]));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(1);
  });

  it("refreshes physical pages after pageInventoryRev on both create and delete", async () => {
    let names = ["test"];
    let rev = 1;
    backendMock.pageInventory.mockImplementation(async () =>
      inventory(rev++, ...names.map(physical), referenced("test/linked")));
    const { allPages, bumpPageInventoryRev } = await loadInventory();

    await vi.waitFor(() => expect(allPages()).toEqual([page("test")]));
    backendMock.pageInventory.mockClear();
    names = ["created", "test"];
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(allPages()).toEqual([page("created"), page("test")]));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(1);

    backendMock.pageInventory.mockClear();
    names = ["test"];
    bumpPageInventoryRev();
    await vi.waitFor(() => expect(allPages()).toEqual([page("test")]));
    expect(backendMock.pageInventory).toHaveBeenCalledTimes(1);
  });

  it("rejects physical and reference-name responses from a superseded graph", async () => {
    const resolvers: Array<(value: PageInventory) => void> = [];
    backendMock.pageInventory.mockImplementation(() => new Promise<PageInventory>((resolve) => {
      resolvers.push(resolve);
    }));
    const { allPageNames, bumpGraphEpoch } = await loadInventory();

    await vi.waitFor(() => expect(allPageNames()).toEqual([]));
    await vi.waitFor(() => expect(resolvers).toHaveLength(1));
    bumpGraphEpoch();
    await vi.waitFor(() => expect(resolvers).toHaveLength(2));

    resolvers[1](inventory(1, physical("fresh"), referenced("fresh/child")));
    await vi.waitFor(() => expect(allPageNames()).toEqual(["fresh", "fresh/child"]));

    resolvers[0](inventory(5, physical("stale"), referenced("stale/child")));
    await Promise.resolve();
    await Promise.resolve();
    expect(allPageNames()).toEqual(["fresh", "fresh/child"]);
  });
});
