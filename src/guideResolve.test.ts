import { afterEach, describe, expect, it } from "vitest";
import {
  doc,
  loadFeed,
  loadGuidePages,
  resetStore,
  resolveGuideBlockRef,
  resolveGuidePageDto,
} from "./store";
import type { BlockDto, PageDto } from "./types";

// F2: the in-app Guide is virtual (never on disk), so the backend `((uuid))` /
// `{{embed [[page]]}}` resolvers can't see it. resolveGuideBlockRef /
// resolveGuidePageDto are the in-memory fallback, consulted ONLY on a backend
// miss. Necessity: they must resolve loaded GUIDE pages but stay null for real
// (non-guide) pages, or a real-graph ref would resolve to a stale guide block.

const FEED = "00000000-0000-4000-8000-00000000feed";
const REMOTE = "00000000-0000-4000-8000-000000000abc";

function block(id: string, raw: string, children: BlockDto[] = []): BlockDto {
  return { id, raw, collapsed: false, children };
}
function guidePage(name: string, title: string, blocks: BlockDto[]): PageDto {
  return { name, kind: "page", title, pre_block: null, blocks, format: "md" };
}

afterEach(() => resetStore());

describe("virtual-guide resolution (F2)", () => {
  it("resolves a block ref whose store id equals the uuid", () => {
    loadGuidePages([
      guidePage("Tine-guide/Feature showcase", "Feature showcase", [
        block(FEED, `This block is a reference target.\nid:: ${FEED}`),
      ]),
    ]);
    const g = resolveGuideBlockRef(FEED);
    expect(g).not.toBeNull();
    expect(g!.page).toBe("Tine-guide/Feature showcase");
    expect(g!.kind).toBe("page");
    expect(g!.blocks[0].raw).toContain("This block is a reference target.");
  });

  it("resolves a ref target that carries id:: even when its store key differs (dup case)", () => {
    // Store key `n1` ≠ the persisted id:: — mirrors the cross-page dup guard that
    // re-keys a block while leaving its raw id:: intact.
    loadGuidePages([
      guidePage("Tine-guide/Project/Roadmap", "Project/Roadmap", [
        block("n1", `The roadmap bullet.\nid:: ${REMOTE}`),
      ]),
    ]);
    const g = resolveGuideBlockRef(REMOTE);
    expect(g?.blocks[0].raw).toContain("The roadmap bullet.");
  });

  it("returns null for an unknown id and for a non-guide page's block", () => {
    loadFeed([
      // A NORMAL (non-guide) page with an id:: block: the disk resolver owns it,
      // so the guide fallback must NOT claim it.
      guidePage("Real page", "Real page", [
        block(FEED, `A real on-disk block.\nid:: ${FEED}`),
      ]),
    ]);
    expect(resolveGuideBlockRef(FEED)).toBeNull();
    expect(resolveGuideBlockRef("nonexistent-id")).toBeNull();
  });

  it("resolves a page embed by bare title for a loaded guide page only", () => {
    loadGuidePages([
      guidePage("Tine-guide/Features/Tips & shortcuts", "Features/Tips & shortcuts", [
        block("t1", "Slash commands and shortcuts."),
      ]),
    ]);
    const p = resolveGuidePageDto("Features/Tips & shortcuts");
    expect(p?.name).toBe("Tine-guide/Features/Tips & shortcuts");
    expect(p?.blocks[0].raw).toContain("Slash commands");
    expect(resolveGuidePageDto("Unknown page")).toBeNull();
  });

  it("page-embed fallback stays null for a non-guide page of the same title", () => {
    loadFeed([guidePage("Features/Tips & shortcuts", "Features/Tips & shortcuts", [block("x", "real")])]);
    expect(resolveGuidePageDto("Features/Tips & shortcuts")).toBeNull();
    // sanity: the page did load as a real (non-guide) page
    expect(doc.pages.some((p) => p.title === "Features/Tips & shortcuts" && !p.guide)).toBe(true);
  });
});
