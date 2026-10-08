import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { applySidebarSession, rightSidebar, setRightSidebar } from "../ui";
import { favorites, seedFavorites } from "../favorites";
import { RightSidebar } from "./RightSidebar";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.restoreAllMocks(); applySidebarSession({ right: false, items: [] }); setRightSidebar([]); resetStore(); document.body.innerHTML = ""; });

it("shows a failed sidebar page load instead of a permanent spinner", async () => {
  vi.spyOn(backend(), "getPage").mockRejectedValue(new Error("disk unreadable"));
  applySidebarSession({ right: true, items: [{ kind: "page", name: "Missing", pageKind: "page" }] });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <RightSidebar />, root);
  await vi.waitFor(() => expect(root.textContent).toContain("Could not load"));
  dispose();
});

it("opening an alias in the sidebar follows its owner without rewriting the favorites config (I-9)", async () => {
  seedFavorites(["Nickname"]);
  const write = vi.spyOn(backend(), "setFavorites").mockResolvedValue();
  vi.spyOn(backend(), "getBacklinks").mockResolvedValue([]);
  vi.spyOn(backend(), "getUnlinkedRefs").mockResolvedValue([]);
  vi.spyOn(backend(), "getBlockRefCounts").mockResolvedValue({});
  vi.spyOn(backend(), "getPage").mockResolvedValue({
    name: "Real Page", kind: "page", title: "Real Page", pre_block: null, id: "pages/Real Page.md", rev: null,
    blocks: [{ id: "owner-block", raw: "owner content", collapsed: false, children: [] }],
  } as never);
  applySidebarSession({ right: true, items: [{ kind: "page", name: "Nickname", pageKind: "page" }] });
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <RightSidebar />, root);
  try {
    await vi.waitFor(() => expect(rightSidebar()[0]).toMatchObject({ kind: "page", name: "Real Page" }));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(write).not.toHaveBeenCalled();
    expect(favorites().map((f) => f.name)).toEqual(["Nickname"]);
  } finally { dispose(); seedFavorites([]); }
});
