import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { InlineText } from "./inline";
import { initParser } from "./parse";
import { backend } from "../backend";
import * as panes from "../panes";

vi.mock("../nativeChrome", async (original) => ({ ...await original<typeof import("../nativeChrome")>(), isMobilePlatform: true }));
beforeAll(initParser);
afterEach(() => { vi.restoreAllMocks(); document.body.replaceChildren(); });

it.each([
  ["![paper](file:///tmp/paper.pdf)", "md"],
  ["[[file:///tmp/paper.pdf][paper]]", "org"],
] as const)("mobile keeps the existing outside-link action: %s", (source, format) => {
  const external = vi.spyOn(backend(), "openExternal").mockResolvedValue();
  const reader = vi.spyOn(panes, "openPdf").mockReturnValue(null);
  const host = document.createElement("div"); document.body.append(host);
  const dispose = render(() => <InlineText text={source} format={format} />, host);
  try {
    expect(host.querySelector(".pdf-link")).toBeNull();
    host.querySelector<HTMLAnchorElement>("a")!.click();
    expect(external).toHaveBeenCalledExactlyOnceWith("file:///tmp/paper.pdf");
    expect(reader).not.toHaveBeenCalled();
  } finally { dispose(); }
});
