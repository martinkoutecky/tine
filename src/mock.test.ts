import { describe, it, expect } from "vitest";
import { mockBackend, mockRefusalMessage } from "./mock";
import { mockFileTimes, mockGraphFiles } from "./mockGraph";
import type { PageMail } from "./document";
import type { PageDto } from "./types";

// The browser mock answers graph questions through the core over its fixture
// files (ADR 0081); these pin the mock's own parts: fixtures, edits, refusals
// and host state. The core's answers have their own tests.
describe("mock backend", () => {
  it("accepts the caller's exact asset cap and refuses one byte over (L-1)", async () => {
    const api = mockBackend();
    const TEST_ASSET_MAX_BYTES = 5;
    const TOO_SMALL_MAX_BYTES = TEST_ASSET_MAX_BYTES - 1;
    const name = await api.saveAsset("l1-limit.bin", new Uint8Array(TEST_ASSET_MAX_BYTES), 0);
    expect((await api.readAsset(name, TEST_ASSET_MAX_BYTES)).byteLength).toBe(TEST_ASSET_MAX_BYTES);
    await expect(api.readAsset(name, TOO_SMALL_MAX_BYTES)).rejects.toBe("asset-too-large");
  });

  it("opens the fixture graph with its config, journals by day and namespaces by file name", async () => {
    const api = mockBackend({ search: "" });
    const graph = await api.loadGraph("");
    if (graph.kind !== "loaded") throw new Error("not loaded");
    expect(graph.meta.journal_page_title_format).toBe("MMM do, yyyy");
    expect(graph.meta.macros.poem).toBe("Roses are $1, violets are $2.");
    expect(graph.config_problem).toBeNull();
    expect((await api.getPage("Jun 14th, 2026", "journal"))?.id).toBe("journals/2026_06_14.md");
    expect((await api.getPage("Formula1/2026", "page"))?.id).toBe("pages/Formula1___2026.md");
    const backlinks = await api.getBacklinks("Tine");
    expect(backlinks.some((group) => group.page === "Jun 14th, 2026")).toBe(true);
  });

  it("selects flag fixtures by the URL and generates the sized ones", () => {
    const base = mockGraphFiles("");
    expect(base.has("logseq/config.edn")).toBe(true);
    expect(base.has("pages/Project Plan.md")).toBe(false);
    const flagged = mockGraphFiles("?conflicts&regressions&big&blocks=3");
    expect(flagged.has("pages/Project Plan.md")).toBe(true);
    expect(flagged.has("pages/Rendering regression.md")).toBe(true);
    expect(flagged.get("pages/Big.md")?.match(/^\t?- /gm)).toHaveLength(3);
  });

  it("gives each file one time: journals newest first, then the rest by path", async () => {
    const times = mockFileTimes(["pages/b.md", "journals/2026_06_12.md", "pages/a.md", "journals/2026_06_14.md"]);
    expect(Object.entries(times).sort(([, a], [, b]) => b - a).map(([path]) => path))
      .toEqual(["journals/2026_06_14.md", "journals/2026_06_12.md", "pages/a.md", "pages/b.md"]);
    const { modified } = await mockBackend({ search: "" }).pageModified();
    const newest = [...modified].sort(([, a], [, b]) => b - a)[0];
    expect(newest).toEqual(["journals/2026_06_14.md", Date.UTC(2026, 5, 15, 12) / 1000]);
  });

  it("writes a submitted page through the core, so a read and an Open answer the saved text", async () => {
    const api = mockBackend({ search: "", files: { "pages/p.md": "- original\n" } });
    const mail: PageMail[] = [];
    await api.onPageMail((m) => mail.push(m));
    const before = await api.getPage("p", "page");
    if (!before) throw new Error("no fixture page");
    const edited: PageDto = { ...before, blocks: [{ ...before.blocks[0], raw: "edited" }] };
    expect(await api.pageSubmit(1, 1, "pages/p.md", edited, 1, null, {} as never)).toBeNull();
    expect((await api.getPage("p", "page"))?.blocks.map((block) => block.raw)).toEqual(["edited"]);
    expect((await api.search("edited", 10)).map((group) => group.page)).toEqual(["p"]);
    await api.pageOpen(1, 2, { name: "p", kind: "page", path: "pages/p.md" });
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const open = mail.find((m) => m.answer?.id === 2)?.page?.text;
    expect(open?.kind === "page" ? open.dto.blocks[0].raw : null).toBe("edited");
    expect(mail.find((m) => m.answer?.id === 1)?.answer?.took).toBe(true);
    // A window reload reloads the preview, and with it the fixture graph.
    await api.pageWindowReloaded();
    expect((await api.getPage("p", "page"))?.blocks.map((block) => block.raw)).toEqual(["original"]);
  });

  it("deletes a page file with replace", async () => {
    const api = mockBackend({ search: "", files: { "pages/gone.md": "- bye\n" } });
    expect(await api.pageDelete(1, "gone", "page")).toBe("applied");
    expect(await api.getPage("gone", "page")).toBeNull();
  });

  it("refuses the store's write paths beyond one page's text, and leaves native-only members absent", async () => {
    const api = mockBackend({ search: "" });
    for (const refused of [
      () => api.renamePage("Tine", "Tine 2", {} as never),
      () => api.renameWouldChange("Tine", "Tine 2", []),
      () => api.mergePages("a", "b", {} as never),
      () => api.resolveSyncConflict("a", "b", {}, "", "", {} as never),
      () => api.copyGuideIntoGraph("Tine Guide", "replace-page"),
      () => api.pagePrintHtml("Tine", {} as never, []),
    ]) await expect(refused()).rejects.toThrow(mockRefusalMessage());
    expect(api.pdfAssetSource).toBeUndefined();
    expect("rescanGraphNow" in api).toBe(false);
    // Journal filename migration: absent, so Settings shows no panel for it.
    expect(api.listJournalFilenameMigrations).toBeUndefined();
    expect(api.applyJournalFilenameMigrations).toBeUndefined();
  });

  it("reads the conflict demo's journal files from the core", async () => {
    const api = mockBackend({ search: "?conflicts" });
    expect(await api.readJournalFile("Friday, 26-06-2026.org")).toBe("* something something\n*\n");
    await expect(api.readJournalFile("absent.org")).rejects.toThrow();
    await expect(api.readJournalFile("../pages/Tine.md")).rejects.toThrow();
  });

  // Blocker 2's core-backed twin: the core's page reads carry a null
  // revision; the host's revision from Open deletes exactly that file.
  it("deletes the exact file the preview opened", async () => {
    const api = mockBackend({ search: "", files: { "pages/gone.md": "- bye\n" } });
    const mail: PageMail[] = [];
    await api.onPageMail((m) => mail.push(m));
    await api.pageOpen(1, 1, { name: "gone", kind: "page", path: "pages/gone.md" });
    await new Promise<void>((resolve) => setTimeout(resolve, 0));
    const text = mail.find((m) => m.answer?.id === 1)?.page?.text;
    const rev = text?.kind === "page" ? text.dto.rev : null;
    expect(rev).toBeTruthy();
    expect(await api.pageDeleteFile(1, "pages/gone.md", rev!)).toBe("applied");
    expect(await api.getPage("gone", "page")).toBeNull();
  });

  it("refuses deleting by a name two files claim, and keeps both", async () => {
    const api = mockBackend({ search: "", files: { "pages/same.md": "- md\n", "pages/same.org": "* org\n",
      "journals/2026_06_20.md": "- md day\n", "journals/2026_06_20.org": "* org day\n" } });
    expect(await api.pageDelete(1, "same", "page")).toBe("refused");
    expect(await api.pageDelete(1, "same", "page", "pages/same.md")).toBe("refused");
    expect(await api.pageDelete(1, "Jun 20th, 2026", "journal")).toBe("refused");
    expect((await api.getPageByPath("pages/same.md"))?.id).toBe("pages/same.md");
    expect((await api.getPageByPath("pages/same.org"))?.id).toBe("pages/same.org");
    expect((await api.getPageByPath("journals/2026_06_20.md"))?.id).toBe("journals/2026_06_20.md");
  });

  // Round 2 blocker 1: a delete never erases a submit the preview accepted,
  // whichever request reaches the core first. Markdown and Org, page and journal.
  const RACED = [
    { path: "pages/race.md", text: "- original\n", name: "race", kind: "page" as const },
    { path: "pages/race.org", text: "* original\n", name: "race", kind: "page" as const },
    { path: "journals/2026_06_21.md", text: "- original\n", name: "Jun 21st, 2026", kind: "journal" as const },
    { path: "journals/2026_06_21.org", text: "* original\n", name: "Jun 21st, 2026", kind: "journal" as const },
  ];
  const tick = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
  async function opened(file: (typeof RACED)[number]) {
    const api = mockBackend({ search: "", files: { [file.path]: file.text } });
    const mail: PageMail[] = [];
    await api.onPageMail((m) => mail.push(m));
    await api.pageOpen(1, 1, { name: file.name, kind: file.kind, path: file.path });
    await tick();
    const text = mail.find((m) => m.answer?.id === 1)?.page?.text;
    if (text?.kind !== "page") throw new Error("no opened page");
    const edited: PageDto = { ...text.dto, blocks: [{ ...text.dto.blocks[0], raw: "submitted" }] };
    const raws = async () => (await api.getPageByPath(file.path))?.blocks.map((block) => block.raw) ?? null;
    return { api, rev: text.dto.rev!, edited, raws };
  }
  for (const file of RACED) {
    it(`keeps an accepted submit that races an exact delete (${file.path})`, async () => {
      for (const submitFirst of [true, false]) {
        const { api, rev, edited, raws } = await opened(file);
        const submit = () => api.pageSubmit(1, 2, file.path, edited, 1, null, {} as never);
        const remove = () => api.pageDeleteFile(1, file.path, rev);
        const [submitted, removed] = submitFirst ? [submit(), remove()] : (() => { const r = remove(); return [submit(), r]; })();
        // First come, first served: a submit first moves the version the delete was typed on.
        expect(await removed).toBe(submitFirst ? "refused" : "applied");
        if (submitFirst) expect(await submitted).toBeNull();
        if (await submitted === null) expect(await raws()).toEqual(["submitted"]);
      }
    });

    it(`keeps an accepted submit that races a delete by name (${file.path})`, async () => {
      const { api, edited, raws } = await opened(file);
      const removed = api.pageDelete(1, file.name, file.kind, file.path);
      const submitted = api.pageSubmit(1, 2, file.path, edited, 1, null, {} as never);
      expect(await removed).toBe("applied");
      // The submit then writes the page again: what it accepted is what the graph holds.
      expect(await submitted).toBeNull();
      expect(await raws()).toEqual(["submitted"]);
    });

    it(`refuses a by-name delete pending across a reload, keeping the new session's submit (${file.path})`, async () => {
      const { api, edited, raws } = await opened(file);
      const removed = api.pageDelete(1, file.name, file.kind, file.path);
      const { session } = await api.pageWindowReloaded();
      expect(await api.pageSubmit(session, 2, file.path, edited, 1, null, {} as never)).toBeNull();
      expect(await removed).toBe("refused");
      expect(await raws()).toEqual(["submitted"]);
    });
  }

  // Round 2 follow-up 4: a request outlived by a window reload refuses, and
  // sends no mail, instead of answering success with its mail suppressed.
  it("refuses an Open or Discard that a window reload outlived, with no mail", async () => {
    const api = mockBackend({ search: "", files: { "pages/p.md": "- original\n" } });
    const mail: PageMail[] = [];
    await api.onPageMail((m) => mail.push(m));
    const open = api.pageOpen(1, 1, { name: "p", kind: "page", path: null });
    const discard = api.pageDiscard(1, 2, "pages/p.md");
    await api.pageWindowReloaded();
    expect(await open).toEqual({ reason: "not-admitted" });
    expect(await discard).toEqual({ reason: "not-admitted" });
    await tick();
    expect(mail).toEqual([]);
  });

  // Blocker 1's core-backed twin: a move is refused before the source is
  // written, so reopening the source answers its unchanged text.
  it("refuses a cross-page move and leaves the source's text", async () => {
    const api = mockBackend({ search: "", files: { "pages/src.md": "- keep\n- move\n", "pages/dst.md": "- there\n" } });
    const source = (await api.getPage("src", "page"))!;
    const receiver = (await api.getPage("dst", "page"))!;
    const refusal = await api.pageMove(1, 1, ["pages/src.md", { ...source, blocks: [source.blocks[0]] }, 1],
      ["pages/dst.md", { ...receiver, blocks: [...receiver.blocks, source.blocks[1]] }, 1], {} as never);
    expect(refusal).toEqual({ reason: "failed", message: mockRefusalMessage() });
    expect((await api.getPage("src", "page"))?.blocks.map((block) => block.raw)).toEqual(["keep", "move"]);
    expect((await api.getPage("dst", "page"))?.blocks.map((block) => block.raw)).toEqual(["there"]);
  });
});
