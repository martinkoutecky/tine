import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "./parse";
import { InlineText } from "./inline";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.restoreAllMocks(); document.body.innerHTML = ""; });
const id = (n: number) => `00000000-0000-0000-0000-${String(n).padStart(12, "0")}`;

it("bounds a hostile block-reference chain and allows many shallow references", async () => {
  vi.spyOn(backend(), "resolveBlocks").mockImplementation(async (ids) => ids.map((key) => {
    const n = Number(key.slice(-12));
    return { page: "P", kind: "page", blocks: [{
      id: key, raw: n < 20 ? `((${id(n + 1)}))` : "end", collapsed: false, children: [],
    }] };
  }));
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text={`((${id(0)}))`} />, host);
  await vi.waitFor(() => expect(host.querySelector(".link-depth-warning"), host.innerHTML).not.toBeNull());
  dispose();

  const shallow = document.createElement("div");
  document.body.append(shallow);
  const disposeShallow = render(() => <InlineText text={Array(50).fill(`((${id(20)}))`).join(" ")} />, shallow);
  await vi.waitFor(() => expect(shallow.querySelectorAll(".block-ref").length).toBe(50));
  expect(shallow.querySelector(".link-depth-warning")).toBeNull();
  disposeShallow();
});
