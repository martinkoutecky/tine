import { afterEach, beforeAll, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { initParser } from "../render/parse";
import { resetStore } from "../document";
import { resetTabsToJournals } from "../router";
import { PageView } from "./Page";

beforeAll(async () => { await initParser(); });
afterEach(() => { vi.restoreAllMocks(); resetStore(); resetTabsToJournals(); document.body.innerHTML = ""; });

it("shows a failed initial journal feed load", async () => {
  resetTabsToJournals();
  vi.spyOn(backend(), "journalFeedPage").mockRejectedValue(new Error("disk unreadable"));
  const root = document.createElement("div");
  document.body.append(root);
  const dispose = render(() => <PageView />, root);
  await vi.waitFor(() => expect(root.textContent).toContain("Couldn't open"));
  expect(root.textContent).toContain("disk unreadable");
  expect(root.querySelector(".page-loading")).toBeNull();
  dispose();
});
