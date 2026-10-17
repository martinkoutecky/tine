// A page the host creates is held in the format of the file it was written to
// (ORG-CREATED-FORMAT). On an Org graph the host writes a new page to `.org`
// (the store resolves an absent page with `:preferred-format`), but callers
// such as the quick switcher and Customize Today's sections build the DTO
// without `format`; the working copy was then held as Markdown, so a property
// set on the freshly created page was spelled `key:: value` into the `.org`
// file, where Logseq and Tine never read it back as a property.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { backend } from "../backend";
import { endEdit, startEditing } from "../editorController";
import { setGraphMeta } from "../graphSession";
import { seedSectionsPage } from "../journalSections";
import type { GraphMeta, PageDto } from "../types";
import { emptyPage, switcherPage } from "./convert";
import { setBlockProperty, setPageProperty } from "./edits/properties";
import { createPage, flushPage } from "./host/wiring";
import { bindTestHost, submittedPages } from "./host/wiring.test.support";
import { doc, pageByName } from "./model";
import { loadSingle, resetStore } from "./workingSet";

/** An Org graph: the host resolves an absent page to an `.org` file, as
 *  `Store::resolve_page` does with `:preferred-format :org`. */
function orgGraph() {
  setGraphMeta({ preferred_format: "org", default_journal_queries: [], disable_scheduled_and_deadline_query: true } as unknown as GraphMeta);
  const b = backend();
  const open = b.pageOpen.bind(b);
  vi.spyOn(b, "pageOpen").mockImplementation((session, id, page) =>
    open(session, id, page.path !== null ? page : { ...page, path: `pages/${page.name}.org` }));
}

/** The last DTO the window submitted for `name` (what the host saves). */
function lastSubmitted(submit: { mock: { calls: unknown[][] } }, name: string): PageDto {
  const sent = submittedPages(submit, name);
  return sent[sent.length - 1]!;
}

beforeEach(async () => {
  resetStore();
  await bindTestHost();
  orgGraph();
});
afterEach(() => {
  resetStore();
  setGraphMeta(null);
  vi.restoreAllMocks();
});

describe("a page created on an Org graph edits as Org", () => {
  it("quick switcher: a block property on the new page is an Org :PROPERTIES: drawer", async () => {
    await createPage("Fresh", switcherPage("Fresh"));
    const page = pageByName("Fresh")!;
    const submit = vi.spyOn(backend(), "pageSubmit");
    const root = page.roots[0]!;
    setBlockProperty(root, "id", "abc");
    expect(doc.byId[root]!.raw).toContain(":PROPERTIES:\n:id: abc\n:END:");
    expect(doc.byId[root]!.raw).not.toContain("id:: abc");
    await flushPage("Fresh");
    const saved = lastSubmitted(submit, "Fresh");
    expect(saved.blocks[0]!.raw).toContain(":PROPERTIES:\n:id: abc\n:END:");
  });

  it("quick switcher: a page property on the new page is an Org #+key: line", async () => {
    await createPage("Fresh", switcherPage("Fresh"));
    const submit = vi.spyOn(backend(), "pageSubmit");
    setPageProperty("Fresh", "foo", "bar");
    expect(pageByName("Fresh")!.preBlock ?? "").toContain("#+foo: bar");
    await flushPage("Fresh");
    const saved = lastSubmitted(submit, "Fresh");
    expect(saved.pre_block ?? "").toContain("#+foo: bar");
    expect(saved.pre_block ?? "").not.toContain("foo:: bar");
  });

  it("Customize Today's sections: properties on the new sections page are Org", async () => {
    await createPage("Journal sections", seedSectionsPage("Journal sections", "org", "{{agenda}}"));
    const page = pageByName("Journal sections")!;
    const submit = vi.spyOn(backend(), "pageSubmit");
    const root = page.roots[0]!;
    setBlockProperty(root, "id", "abc");
    setPageProperty("Journal sections", "foo", "bar");
    await flushPage("Journal sections");
    const saved = lastSubmitted(submit, "Journal sections");
    expect(saved.blocks[0]!.raw).toContain(":PROPERTIES:\n:id: abc\n:END:");
    expect(saved.pre_block).toBe("#+foo: bar\n#+tine/journal-sections: true");
  });

  it("a page already shown as an empty placeholder (a link to a page with no file) is created as Org", async () => {
    loadSingle(emptyPage("Fresh", "page"));
    await createPage("Fresh", switcherPage("Fresh"));
    const root = pageByName("Fresh")!.roots[0]!;
    setBlockProperty(root, "id", "abc");
    expect(doc.byId[root]!.raw).toContain(":PROPERTIES:\n:id: abc\n:END:");
  });

  it("the file's extension wins over a DTO that names another format", async () => {
    await createPage("Fresh", { ...switcherPage("Fresh"), format: "md" });
    const root = pageByName("Fresh")!.roots[0]!;
    setBlockProperty(root, "id", "abc");
    expect(doc.byId[root]!.raw).toContain(":PROPERTIES:\n:id: abc\n:END:");
  });
});

describe("a page with no file on an Org graph edits as Org", () => {
  it("a routed page that has no file yet, once its block is edited, takes the format of the file the host resolved", async () => {
    loadSingle(emptyPage("Ghost", "page"));
    // Entering editing opens the page on the host; its "no file" answer names the file.
    startEditing(pageByName("Ghost")!.roots[0]!, 0);
    await vi.waitFor(() => expect(pageByName("Ghost")!.id).toBe("pages/Ghost.org"));
    endEdit("blur");
    const root = pageByName("Ghost")!.roots[0]!;
    setBlockProperty(root, "id", "abc");
    expect(doc.byId[root]!.raw).toContain(":PROPERTIES:\n:id: abc\n:END:");
  });
});
