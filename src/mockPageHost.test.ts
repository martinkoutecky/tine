import { describe, expect, it } from "vitest";
import type { PageMail } from "./document";
import { RetiredGraph, type MockGraphSession } from "./mockGraph";
import { mockPageHost, type PageHostGraph } from "./mockPageHost";
import type { PageDto, PageRead, ResolvedPage } from "./types";

const page = (raw: string): PageRead =>
  ({ id: "pages/p.md", name: "p", kind: "page", title: "p", pre_block: null,
    blocks: [{ id: "b", raw, collapsed: false, children: [] }], rev: "r1" }) as unknown as PageRead;

/** A graph that keeps the last submitted page per path and each file's
 * version, as the core's memory graph does (its page reads carry no
 * revision), one session per window load. `hold` holds each submit before it
 * reaches the core (the core's open) until the returned release runs. */
function host(refuse = false, hold = false) {
  const held: (() => void)[] = [];
  /** Each submit the graph's core ran: the session and the version it was typed on. */
  const submits: [number, number][] = [];
  const state = { resolved: { kind: "existing", id: "pages/p.md", others: [] } as ResolvedPage };
  const begin = (session: number) => {
    const files = new Map<string, PageRead>([["pages/p.md", { ...page("original"), rev: null } as unknown as PageRead]]);
    const versions = new Map<string, number>();
    const version = (key: string) => versions.get(key) ?? 1;
    const wrote = (key: string) => versions.set(key, version(key) + 1);
    let retired = false;
    const use = async <T,>(answer: () => T, wait = false): Promise<T> => {
      if (wait) await new Promise<void>((resolve) => held.push(resolve));
      if (retired) throw new RetiredGraph();
      return answer();
    };
    const s: MockGraphSession & { retire(): void } = {
      session,
      get retired() { return retired; },
      read: () => Promise.reject(new Error("no reads")),
      page: (path) => use(() => ({ page: files.get(path) ?? null, version: version(path) })),
      resolve: () => use(() => state.resolved),
      submit: (key, dto) => use(() => {
        submits.push([session, version(key)]);
        if (refuse) return { refused: { reason: "read-only" as const, message: "no" } };
        files.set(key, { ...(dto as PageRead), id: key });
        wrote(key);
        return { version: version(key) };
      }, hold),
      deleteFile: (path, at) => use(() => {
        if (at !== version(path) || !files.has(path)) return false;
        files.delete(path);
        wrote(path);
        return true;
      }),
      deleteName: (_name, _kind, expected) => use(() => {
        const target = state.resolved;
        if (target.kind !== "existing" || target.others.length > 0 || (expected !== undefined && expected !== target.id)) return false;
        files.delete(target.id);
        wrote(target.id);
        return true;
      }),
      write: (path, text) => use(() => {
        if (text === null) files.delete(path);
        else files.set(path, page(text.replace(/^- /, "").trimEnd()));
        wrote(path);
        return undefined;
      }),
      retire() { retired = true; },
    };
    return { s, files };
  };
  let current = begin(1);
  const graph: PageHostGraph = {
    current: () => current.s,
    reset() { current.s.retire(); current = begin(current.s.session + 1); },
  };
  const h = mockPageHost(graph, () => "preview refusal");
  const mail: PageMail[] = [];
  void h.onPageMail((m) => mail.push(m));
  return { h, mail, submits, held, state, files: () => current.files };
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

  it("names the graph's version in the submit mail and the publication wait", async () => {
    const { h, mail, submits } = host();
    await h.pageSubmit(1, 1, "pages/p.md", page("one") as PageDto);
    await h.pageSubmit(1, 2, "pages/p.md", page("two") as PageDto);
    await flush();
    expect(submits).toEqual([[1, 1], [1, 2]]);
    expect(mail.find((m) => m.answer?.id === 2)?.page?.version).toBe(3);
    const took = mail.find((m) => m.answer?.id === 2)?.page?.text;
    const rev = took?.kind === "unchanged" ? took.rev : undefined;
    expect(rev).toBeTruthy();
    expect(await h.pageWait(1, [{ key: "pages/p.md", version: 3 }])).toEqual([{ key: "pages/p.md", version: 3, rev }]);
    expect(await h.pageWait(1, [{ key: "pages/p.md", version: 4 }])).toEqual([]);
  });

  // ADR 0081 Decision 4: the preview writes one page per request. A move is
  // refused before either endpoint's submit, so the source never loses a
  // block the receiver would then refuse.
  it("refuses a cross-page move before writing either endpoint", async () => {
    const { h, mail, submits } = host();
    expect(await h.pageMove(1, 1, ["pages/p.md", page("source") as PageDto, 1], ["pages/q.md", page("receiver") as PageDto, 1]))
      .toEqual({ reason: "failed", message: "preview refusal" });
    await h.pageOpen(1, 2, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    expect(submits).toEqual([]);
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

  // A revision names its session: one from before a reload deletes nothing after it.
  it("refuses an exact delete at a revision of an earlier session", async () => {
    const { h, mail, files } = host();
    await h.pageOpen(1, 1, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    const opened = mail.find((m) => m.answer?.id === 1)?.page;
    const rev = opened?.text.kind === "page" ? opened.text.dto.rev : null;
    await h.pageWindowReloaded();
    expect(await h.pageDeleteFile(2, "pages/p.md", rev!)).toBe("refused");
    expect(files().has("pages/p.md")).toBe(true);
  });

  // Same-title Markdown and Org pages, or a day's two journals: the core
  // answers both claimants. Deleting by name takes neither.
  it("refuses deleting a name another file also claims", async () => {
    const { h, state, files } = host();
    state.resolved = { kind: "existing", id: "pages/p.md", others: ["pages/p.org"] };
    expect(await h.pageDelete(1, "p", "page")).toBe("refused");
    expect(await h.pageDelete(1, "p", "page", "pages/p.md")).toBe("refused");
    expect(files().has("pages/p.md")).toBe(true);
    state.resolved = { kind: "existing", id: "pages/p.md", others: [] };
    expect(await h.pageDelete(1, "p", "page", "pages/p.md")).toBe("applied");
    expect(files().has("pages/p.md")).toBe(false);
  });

  // A request answers only in its session: after a window reload, a submit
  // still pending refuses, sends no mail and never reaches the new graph.
  it("refuses a pending submit after a window reload", async () => {
    const { h, mail, submits, held } = host(false, true);
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
    expect(await next).toBeNull();
    expect(submits).toEqual([[2, 1]]);
  });
});
