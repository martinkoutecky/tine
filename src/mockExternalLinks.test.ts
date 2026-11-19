import { afterEach, expect, it, vi } from "vitest";
import { mockBackend } from "./mock";

// GH #984 review ef9423888: the browser preview shares the published reader's
// handoff check (checkBrowserHandoff): same-site scheme-less links open a tab as
// on the Beta; off-site scheme-less and refused schemes do not.
afterEach(() => vi.unstubAllGlobals());

it("the browser preview opens same-site relative links and refuses off-site or refused ones", async () => {
  const open = vi.fn();
  vi.stubGlobal("window", { open, location: { href: "http://localhost:1420/#/page/Home" } });
  const api = mockBackend();
  for (const url of ["#section", "#/page/Project", "*Heading", "other.html?x=1"]) await api.openExternal(url);
  expect(open.mock.calls.map(([url]) => url)).toEqual(["#section", "#/page/Project", "*Heading", "other.html?x=1"]);
  open.mockClear();
  const refused = await Promise.allSettled(["//example.org", "\\\\example.org", " javascript:alert(1)", "zotero://x"].map((url) => api.openExternal(url)));
  expect(refused.map(({ status }) => status)).toEqual(["rejected", "rejected", "rejected", "rejected"]);
  expect(open).not.toHaveBeenCalled();
});
