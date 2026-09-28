import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { applySidebarSession, setRightSidebar } from "../ui";
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
