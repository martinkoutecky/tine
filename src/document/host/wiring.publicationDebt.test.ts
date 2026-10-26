// J6, second round (STEP3-DESIGN "Manager decisions on REVIEW-3b-P2b", R1): a
// version the host took from this window stays the window's until a true
// publication answer. A page that left every view while its taken text is
// unpublished is not dirty and not busy, yet the working set must keep it, the
// recovery panel must list it ("Saving") and a reload must not replace it.

import { afterEach, expect, it, vi } from "vitest";
import { backend } from "../../backend";
import type { PageDto } from "../../types";
import { resetStore, setRaw } from "../index";
import { pageByName } from "../model";
import { ensurePageLoaded, loadRoutedPage } from "../workingSet";
import { hostBlocksReload, unsavedDrafts } from "./wiring";
import { bindTestHost } from "./wiring.test.support";

afterEach(() => { resetStore(); vi.restoreAllMocks(); });

function dto(name: string): PageDto & { id: string } {
  return { name, kind: "page", title: name, id: `pages/${name}.md`, rev: "r1", pre_block: null,
    blocks: [{ id: `${name}-block`, raw: name, collapsed: false, children: [] }] };
}

it("R1: a taken, unpublished page stays loaded, listed and unreloadable under working-set pressure until it publishes", async () => {
  resetStore();
  loadRoutedPage(dto("P"));
  await bindTestHost();
  const answers: ((published: boolean) => void)[] = [];
  vi.spyOn(backend(), "pageWait").mockImplementation(() => new Promise((resolve) => { answers.push(resolve); }));
  const submit = vi.spyOn(backend(), "pageSubmit");
  setRaw("P-block", "typed", { timetracking: false });
  await vi.waitFor(() => expect(submit).toHaveBeenCalledOnce(), { timeout: 2000 });
  // The host took the text (mock mail); its publication wait is pending.
  await vi.waitFor(() => expect(answers.length).toBeGreaterThan(0), { timeout: 2000 });
  loadRoutedPage(dto("Q"));
  for (let i = 0; i < 81; i += 1) ensurePageLoaded(dto(`Other${i}`));
  expect(pageByName("P")?.name).toBe("P");
  expect(unsavedDrafts().map((draft) => [draft.name, draft.state])).toEqual([["P", "Saving"]]);
  expect(hostBlocksReload("P")).toBe(true);
  for (const answer of answers.splice(0)) answer(true);
  await vi.waitFor(() => expect(unsavedDrafts()).toEqual([]));
  // Published and let go: the working set may evict it now.
  ensurePageLoaded(dto("Other81"));
  expect(pageByName("P")).toBeUndefined();
});
