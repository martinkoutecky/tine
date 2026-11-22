import { beforeEach, describe, expect, it, vi } from "vitest";


const files = {
  "pages/Public.md": "alias:: Alias\n\n- Owner body\n- see [[launch day]]\n",
  "journals/2026_10_26.md": "alias:: launch day\n\n- Launch journal body\n",
};

const holder = vi.hoisted(() => ({ api: null as unknown }));
vi.mock("./backend", () => ({ backend: () => holder.api }));

async function load() {
  await (await import("./render/parse")).initParser();
  const { testPublishedBackend } = await import("./publishedBackend.methods.test.support");
  const api = testPublishedBackend(files);
  holder.api = api;
  const index = await import("./pageIndex");
  return { api, ...index };
}

beforeEach(() => {
  vi.resetModules();
});

describe("published site: alias links use the core inventory", () => {
  it("opens the owner of an ordinary page's alias, as on develop before CM step 2", async () => {
    const { api, installPageIndex, allPageNames, linkTarget, navigationTarget } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(allPageNames()).toContain("Public"));
    // PageRef: what the link opens.
    const link = linkTarget("Alias");
    expect(link.kind).toBe("page");
    // Router: an ordinary-page route without a path goes through navigationTarget.
    const route = link.kind === "page" ? navigationTarget(link.name) : link;
    const page = await api.getPage(route.name, route.kind);
    expect(page?.id).toBe("pages/Public.md");
    expect(page?.blocks[0]?.raw).toBe("Owner body");
  });

  it("opens the journal of a journal's alias on a published site", async () => {
    const { api, installPageIndex, allPageNames, linkTarget, navigationTarget } = await load();
    installPageIndex();
    await vi.waitFor(() => expect(allPageNames()).toContain("Public"));
    const link = linkTarget("launch day");
    expect(link).toEqual({ name: "Oct 26th, 2026", kind: "journal" });
    const route = link.kind === "page" ? navigationTarget(link.name) : link;
    const page = await api.getPage(route.name, route.kind);
    expect(page?.id).toBe("journals/2026_10_26.md");
    expect(page?.blocks[0]?.raw).toBe("Launch journal body");
    // The route names the owner, so its linked references are the journal's own.
    expect((await api.getBacklinks(route.name)).map((group) => group.page)).toEqual(["Public"]);
  });
});
