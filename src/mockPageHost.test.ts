import { describe, expect, it } from "vitest";
import type { PageMail } from "./document";
import { mockPageHost } from "./mockPageHost";
import type { PageDto } from "./types";

const page = (raw: string): PageDto =>
  ({ name: "p", kind: "page", title: "p", pre_block: null, blocks: [{ id: "b", raw, collapsed: false, children: [] }],
    rev: "r1" }) as unknown as PageDto;

function host() {
  const h = mockPageHost((path) => (path === "pages/p.md" ? page("original") : null),
    () => ({ kind: "page", id: "pages/p.md" }) as never);
  const mail: PageMail[] = [];
  void h.onPageMail((m) => mail.push(m));
  return { h, mail };
}

const flush = () => new Promise<void>((resolve) => queueMicrotask(resolve));
const openText = (m: PageMail | undefined) =>
  m?.page?.text.kind === "page" ? m.page.text.dto.blocks[0].raw : null;

describe("mockPageHost", () => {
  // The plugin launch journey (scripts/shot-plugin-docs.mjs) edits a page,
  // leaves it, and edits it again: a reopen that answered the mock graph's
  // original text replaced the window's edits and closed the new editor.
  it("answers an Open after a submit with the text it took, as the native host does", async () => {
    const { h, mail } = host();
    await h.pageOpen(1, 1, { name: "p", kind: "page", path: "pages/p.md" });
    await h.pageSubmit(1, 2, "pages/p.md", page("edited"));
    await h.pageClose(1, 3);
    await h.pageOpen(1, 4, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    expect(openText(mail.find((m) => m.answer?.id === 1))).toBe("original");
    expect(openText(mail.find((m) => m.answer?.id === 4))).toBe("edited");
  });

  it("forgets taken text on a window reload, which loads the mock graph again", async () => {
    const { h, mail } = host();
    await h.pageSubmit(1, 1, "pages/p.md", page("edited"));
    const { session } = await h.pageWindowReloaded();
    await h.pageOpen(session, 2, { name: "p", kind: "page", path: "pages/p.md" });
    await flush();
    expect(openText(mail.find((m) => m.answer?.id === 2))).toBe("original");
  });

  it("keeps both endpoints' texts of a move", async () => {
    const { h, mail } = host();
    await h.pageMove(1, 1, ["pages/p.md", page("source"), 1], ["pages/q.md", page("receiver"), 1]);
    await h.pageOpen(1, 2, { name: "p", kind: "page", path: "pages/p.md" });
    await h.pageOpen(1, 3, { name: "q", kind: "page", path: "pages/q.md" });
    await flush();
    expect(openText(mail.find((m) => m.answer?.id === 2))).toBe("source");
    expect(openText(mail.find((m) => m.answer?.id === 3))).toBe("receiver");
  });
});
