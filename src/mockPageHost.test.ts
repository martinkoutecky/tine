import { describe, expect, it } from "vitest";
import type { PageMail } from "./document";
import { mockPageHost, type PageHostGraph } from "./mockPageHost";
import type { PageDto, PageRead } from "./types";

const page = (raw: string): PageRead =>
  ({ id: "pages/p.md", name: "p", kind: "page", title: "p", pre_block: null,
    blocks: [{ id: "b", raw, collapsed: false, children: [] }], rev: "r1" }) as unknown as PageRead;

/** A graph that keeps the last submitted page per path, as the core's memory
 * graph does (its page reads carry no revision). `hold` holds each submit's
 * answer until the returned release runs. */
function host(refuse = false, hold = false) {
  const files = new Map<string, PageRead>([["pages/p.md", { ...page("original"), rev: null } as unknown as PageRead]]);
  const versions: number[] = [];
  const held: (() => void)[] = [];
  const graph: PageHostGraph = {
    async page(path) { return files.get(path) ?? null; },
    async resolve() { return { kind: "existing", id: "pages/p.md", others: [] } as never; },
    async submit(key, dto, version) {
      versions.push(version);
      if (hold) await new Promise<void>((resolve) => held.push(resolve));
      if (refuse) return { refused: { reason: "read-only", message: "no" } };
      files.set(key, { ...(dto as PageRead), id: key });
      return { version: version + 1 };
    },
    async write(path, text) {
      if (text === null) files.delete(path);
      else files.set(path, page(text.replace(/^- /, "").trimEnd()));
    },
    reset() { files.set("pages/p.md", page("original")); },
  };
  const h = mockPageHost(graph, () => "preview refusal");
  const mail: PageMail[] = [];
  void h.onPageMail((m) => mail.push(m));
  return { h, mail, versions, files, held, graph };
}

const flush = () => new Promise<void>((resolve) => queueMicrotask(resolve));
const openText = (m: PageMail | undefined) =>
  m?.page?.text.kind === "page" ? m.page.text.dto.blocks[0].raw : null;

describe("mockPageHost", () => {
  // The plugin launch journey (scripts/shot-plugin-docs.mjs) edits a page,
  // leaves it, and edits it again: a reopen answers the saved text.
  it("answers an Open after a submit with the graph's saved text", async () => {
    const { h, mail } = host();
    await h.pageOpen(1, 1, { name: "p", kind: "page", path: "pages/p.md" });
    await h.pageSubmit(1, 2, "pages/p.md", page("edited") as PageDto);
    await h.pageClose(1, 3);
    await h.pageOpen(1, 4, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    expect(openText(mail.find((m) => m.answer?.id === 1))).toBe("original");
    expect(openText(mail.find((m) => m.answer?.id === 4))).toBe("edited");
  });

  it("keeps each key's version and passes it to the graph", async () => {
    const { h, mail, versions } = host();
    await h.pageSubmit(1, 1, "pages/p.md", page("one") as PageDto);
    await h.pageSubmit(1, 2, "pages/p.md", page("two") as PageDto);
    await flush();
    expect(versions).toEqual([1, 2]);
    expect(mail.find((m) => m.answer?.id === 2)?.page?.version).toBe(3);
    const took = mail.find((m) => m.answer?.id === 2)?.page?.text;
    const rev = took?.kind === "unchanged" ? took.rev : undefined;
    expect(rev).toBeTruthy();
    expect(await h.pageWait(1, [{ key: "pages/p.md", version: 3 }])).toEqual([{ key: "pages/p.md", version: 3, rev }]);
  });

  // ADR 0081 Decision 4: the preview writes one page per request. A move is
  // refused before either endpoint's submit, so the source never loses a
  // block the receiver would then refuse.
  it("refuses a cross-page move before writing either endpoint", async () => {
    const { h, mail, versions } = host();
    expect(await h.pageMove(1, 1, ["pages/p.md", page("source") as PageDto, 1], ["pages/q.md", page("receiver") as PageDto, 1]))
      .toEqual({ reason: "failed", message: "preview refusal" });
    await h.pageOpen(1, 2, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    expect(versions).toEqual([]);
    expect(openText(mail.find((m) => m.answer?.id === 2))).toBe("original");
    expect(mail.some((m) => m.answer?.id === 1)).toBe(false);
  });

  it("returns the graph's refusal and sends no mail", async () => {
    const { h, mail } = host(true);
    expect(await h.pageSubmit(1, 1, "pages/p.md", page("x") as PageDto)).toEqual({ reason: "read-only", message: "no" });
    await flush();
    expect(mail).toEqual([]);
  });

  // scripts/repro-carry-blur.mjs: an external change arrives under an open window.
  it("delivers an external write as unanswered mail with the new text and a newer version", async () => {
    const { h, mail } = host();
    await h.pageOpen(1, 1, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    await h.externalWrite("pages/p.md", "- outside\n");
    const pushed = mail.at(-1);
    expect(pushed?.answer).toBeNull();
    expect(pushed?.key).toBe("pages/p.md");
    expect(openText(pushed)).toBe("outside");
    expect(pushed?.page?.version).toBe(2);
  });

  // The core's page reads carry no revision: the host's one revision names the
  // file in Open, submit mail, the publication wait and the exact deletion.
  it("deletes a file only at the revision its Open or last submit named", async () => {
    const { h, mail } = host();
    await h.pageOpen(1, 1, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    const opened = mail.find((m) => m.answer?.id === 1)?.page;
    const rev = opened?.text.kind === "page" ? opened.text.dto.rev : null;
    expect(rev).toBeTruthy();
    expect(opened?.disk).toEqual({ kind: "file", rev });
    expect(await h.pageDeleteFile(1, "pages/p.md", "stale")).toBe("refused");
    await h.pageSubmit(1, 2, "pages/p.md", page("edited") as PageDto);
    await flush();
    expect(await h.pageDeleteFile(1, "pages/p.md", rev!)).toBe("refused");
    const took = mail.find((m) => m.answer?.id === 2)?.page?.text;
    const after = took?.kind === "unchanged" ? took.rev : undefined;
    expect(after).toBeTruthy();
    expect(await h.pageDeleteFile(1, "pages/p.md", after!)).toBe("applied");
    expect(await h.pageDeleteFile(1, "pages/p.md", after!)).toBe("refused");
  });

  // Same-title Markdown and Org pages, or a day's two journals: the core
  // answers both claimants. Deleting by name takes neither.
  it("refuses deleting a name another file also claims", async () => {
    const { h, graph, files } = host();
    graph.resolve = async () => ({ kind: "existing", id: "pages/p.md", others: ["pages/p.org"] });
    expect(await h.pageDelete(1, "p", "page")).toBe("refused");
    expect(await h.pageDelete(1, "p", "page", "pages/p.md")).toBe("refused");
    expect(files.has("pages/p.md")).toBe(true);
    graph.resolve = async () => ({ kind: "existing", id: "pages/p.md", others: [] });
    expect(await h.pageDelete(1, "p", "page", "pages/p.md")).toBe("applied");
    expect(files.has("pages/p.md")).toBe(false);
  });

  // A request answers only in its session: after a window reload, a submit
  // still pending sends no mail and moves no version of the new session.
  it("discards a pending submit's answer after a window reload", async () => {
    const { h, mail, versions, held } = host(false, true);
    const submitted = h.pageSubmit(1, 1, "pages/p.md", page("late") as PageDto);
    await flush();
    const { session } = await h.pageWindowReloaded();
    for (const release of held.splice(0)) release();
    expect(await submitted).toEqual({ reason: "not-admitted" });
    await flush();
    expect(mail).toEqual([]);
    const next = h.pageSubmit(session, 2, "pages/p.md", page("fresh") as PageDto);
    await flush();
    for (const release of held.splice(0)) release();
    await next;
    expect(versions).toEqual([1, 1]);
  });
});
