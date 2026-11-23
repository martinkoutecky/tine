import { describe, expect, it } from "vitest";
import type { PageMail } from "./document";
import { mockPageHost, type PageHostGraph } from "./mockPageHost";
import type { PageDto, PageRead } from "./types";

const page = (raw: string): PageRead =>
  ({ id: "pages/p.md", name: "p", kind: "page", title: "p", pre_block: null,
    blocks: [{ id: "b", raw, collapsed: false, children: [] }], rev: "r1" }) as unknown as PageRead;

/** A graph that keeps the last submitted page per path, as the core's memory graph does. */
function host(refuse = false) {
  const files = new Map<string, PageRead>([["pages/p.md", page("original")]]);
  const versions: number[] = [];
  const graph: PageHostGraph = {
    async page(path) { return files.get(path) ?? null; },
    async resolve() { return { kind: "existing", id: "pages/p.md", others: [] } as never; },
    async submit(key, dto, version) {
      versions.push(version);
      if (refuse) return { refused: { reason: "read-only", message: "no" } };
      files.set(key, { ...(dto as PageRead), id: key });
      return { version: version + 1 };
    },
    async remove(path) { files.delete(path); },
    reset() { files.set("pages/p.md", page("original")); },
  };
  const h = mockPageHost(graph);
  const mail: PageMail[] = [];
  void h.onPageMail((m) => mail.push(m));
  return { h, mail, versions };
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
    expect(await h.pageWait(1, [{ key: "pages/p.md", version: 3 }])).toEqual([{ key: "pages/p.md", version: 3, rev: "r1" }]);
  });

  it("keeps both endpoints' texts of a move", async () => {
    const { h, mail } = host();
    await h.pageMove(1, 1, ["pages/p.md", page("source") as PageDto, 1], ["pages/q.md", page("receiver") as PageDto, 1]);
    await h.pageOpen(1, 2, { name: "p", kind: "page", path: "pages/p.md" });
    await h.pageOpen(1, 3, { name: "q", kind: "page", path: "pages/q.md" });
    await flush();
    expect(openText(mail.find((m) => m.answer?.id === 2))).toBe("source");
    expect(openText(mail.find((m) => m.answer?.id === 3))).toBe("receiver");
  });

  it("returns the graph's refusal and sends no mail", async () => {
    const { h, mail } = host(true);
    expect(await h.pageSubmit(1, 1, "pages/p.md", page("x") as PageDto)).toEqual({ reason: "read-only", message: "no" });
    await flush();
    expect(mail).toEqual([]);
  });

  it("deletes a file only at the revision it was read", async () => {
    const { h } = host();
    expect(await h.pageDeleteFile(1, "pages/p.md", "stale")).toBe("refused");
    expect(await h.pageDeleteFile(1, "pages/p.md", "r1")).toBe("applied");
    expect(await h.pageDeleteFile(1, "pages/p.md", "r1")).toBe("refused");
  });
});
