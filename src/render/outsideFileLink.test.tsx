import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import { graphMeta, setGraphMeta } from "../graphSession";

beforeAll(initParser);
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); setGraphMeta(null); });

it("opens an authored outside path as an ordinary file link (GH #457)", () => {
  setGraphMeta({ ...graphMeta()!, root: "/home/user/graph" });
  const open = vi.spyOn(backend(), "openExternal").mockResolvedValue(undefined);
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <InlineText text="[report](files/../../documents/report.pdf)" />, host);
  try {
    const link = host.querySelector<HTMLAnchorElement>("a")!;
    expect(link.hasAttribute("data-broken-file-link")).toBe(false);
    expect(link.title).toBe("/home/user/documents/report.pdf");
    link.click();
    expect(open).toHaveBeenCalledExactlyOnceWith("file:///home/user/documents/report.pdf");
  } finally { dispose(); }
});
