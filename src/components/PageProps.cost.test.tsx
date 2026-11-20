import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { resetStore } from "../document";
import { setDoc } from "../document/model";
import { initParser } from "../render/parse";
import { closePageProps, openPageProps } from "../ui";
import { PageProps } from "./PageProps";

const reads = vi.hoisted(() => ({ opening: 0 }));
vi.mock("../document", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../document")>();
  return { ...actual, pageOpening: (...args: Parameters<typeof actual.pageOpening>) => { reads.opening++; return actual.pageOpening(...args); } };
});

beforeAll(async () => { await initParser(); });
afterEach(() => { closePageProps(); resetStore(); document.body.innerHTML = ""; });

// Review 716a8eb3b (PC-1/PC-4/PC-5): the panel reads the page header once per
// change of the page's source, not once per custom key nor per keystroke.
it("reads a page's header once, whatever its key count and typing", () => {
  const keys = Array.from({ length: 24 }, (_, i) => `custom${i}:: v${i}`);
  setDoc({
    loaded: true, feed: ["P"], byId: {},
    pages: [{ name: "P", kind: "page", title: "P", preBlock: `${keys.join("\n")}\n\nlater:: x\n`, roots: [], format: "md", readOnly: false, guide: false }],
  });
  const host = document.createElement("div");
  document.body.appendChild(host);
  const dispose = render(() => <PageProps />, host);
  reads.opening = 0;
  openPageProps("P", 100, 100);
  expect(host.querySelectorAll(".pp-field").length).toBeGreaterThan(24);
  const opened = reads.opening;
  expect(opened).toBeLessThanOrEqual(1);
  const input = host.querySelector<HTMLInputElement>(".pp-add-key")!;
  for (const typed of ["n", "ne", "new", "new-", "new-k"]) {
    input.value = typed;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }
  expect(reads.opening).toBe(opened);
  dispose();
});
