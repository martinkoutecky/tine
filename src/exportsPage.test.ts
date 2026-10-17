import { readFileSync } from "node:fs";
import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { setGraphMeta } from "./graphSession";
import { EXPORTS_OVERVIEW, ensureExportsPage, exportsMarker, pageSelectorMacro, pageSelectorQuery, selectorPageName } from "./exportsPage";
import selectorFixture from "../crates/tine-core/src/query/fixtures/share/page_selector.json";

const created = vi.hoisted(() => ({ calls: [] as [string, unknown][], refuse: new Set<string>() }));
vi.mock("./document", async (original) => ({
  ...await original<typeof import("./document")>(),
  createPage: vi.fn(async (name: string, dto: unknown) => {
    if (created.refuse.has(name)) throw new Error("conflict");
    created.calls.push([name, dto]);
    return null;
  }),
}));
vi.mock("./owned", async (original) => ({ ...await original<typeof import("./owned")>(), graphOwner: () => () => true }));

afterEach(() => { setGraphMeta(null); vi.restoreAllMocks(); created.calls.length = 0; created.refuse.clear(); });

const absent = null;
function engine(located: string[], read: (name: string) => unknown, format: "md" | "org" = "md") {
  setGraphMeta({ preferred_format: format } as never);
  vi.spyOn(backend(), "parseQuery").mockResolvedValue({ query: { anchor: "page" }, view: {} } as never);
  vi.spyOn(backend(), "queryRun").mockResolvedValue({ anchor: "page", pages: located.map((name) => ({ name })) } as never);
  vi.spyOn(backend(), "getPage").mockImplementation(async (name: string) => read(name) as never);
}

describe("the exports page (contract: export-definitions.md)", () => {
  it("is created on first use with the marker, public:: false and the overview, in Markdown and Org", async () => {
    engine([], () => absent);
    expect(await ensureExportsPage()).toBe("Exports");
    expect(created.calls[0][1]).toMatchObject({
      name: "Exports", format: "md", pre_block: "tine/exports:: true\npublic:: false",
      blocks: [{ raw: "{{query (or (property export) (property tine.share))}}" }],
    });
    created.calls.length = 0;
    engine([], () => absent, "org");
    await ensureExportsPage();
    expect(created.calls[0][1]).toMatchObject({ format: "org", pre_block: "#+tine/exports: true\n#+public: false" });
  });

  it("is found by its marker, so a renamed page is still the exports page", async () => {
    engine(["Shared things"], () => { throw new Error("must not look for a free name"); });
    expect(await ensureExportsPage()).toBe("Shared things");
    expect(created.calls).toEqual([]);
  });

  it("never overwrites a user's page of the default name", async () => {
    engine([], (name) => name === "Exports" ? { name: "Exports", blocks: [] } : absent);
    expect(await ensureExportsPage()).toBe("Exports 2");
    // A file that appears between the name check and the create is a conflict: the next name is tried.
    created.calls.length = 0;
    created.refuse.add("Exports");
    engine([], () => absent);
    expect(await ensureExportsPage()).toBe("Exports 2");
  });

  it("lists every definition with the contract's destination properties", () => {
    const rust = readFileSync("crates/tine-core/src/export_definition.rs", "utf8");
    expect(rust).toContain('DESTINATION_KEYS: [&str; 2] = ["export", "tine.share"]');
    expect(EXPORTS_OVERVIEW).toBe("{{query (or (property export) (property tine.share))}}");
    expect(exportsMarker("md")).toBe("tine/exports:: true\npublic:: false");
    expect(exportsMarker("org")).toBe("#+tine/exports: true\n#+public: false");
  });
});

describe("the this-page selector", () => {
  it("is the wire value the Rust test prints as `@page and name = 'Project'`", () => {
    expect(pageSelectorQuery("Project")).toEqual(selectorFixture);
    expect(selectorPageName(pageSelectorQuery("Project"))).toBe("Project");
    expect(selectorPageName({ ...pageSelectorQuery("Project"), anchor: "block" })).toBeNull();
  });

  it("is spelled by the engine's printer as a tine-query macro", async () => {
    const print = vi.spyOn(backend(), "printQuery").mockResolvedValue("@page and name = 'Project'");
    expect(await pageSelectorMacro("Project")).toBe("{{tine-query @page and name = 'Project'}}");
    expect(print).toHaveBeenCalledWith(pageSelectorQuery("Project"), {}, "tql_macro");
  });
});
