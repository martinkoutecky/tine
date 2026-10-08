import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { resetStore, type FeedPage } from "../document";
import { journalTitle, localDayKey, setCurrentDayKeyForTest } from "../journal";
import { CarryActions } from "./Page";

afterEach(() => {
  setCurrentDayKeyForTest(localDayKey());
  resetStore();
  document.body.innerHTML = "";
});

it("swaps a mounted journal's carry buttons when the local day changes", () => {
  const name = journalTitle(new Date());
  setCurrentDayKeyForTest(localDayKey(new Date(new Date().getFullYear(), new Date().getMonth(), new Date().getDate() - 1)));
  const page: FeedPage = { name, kind: "journal", title: name, preBlock: null, roots: [], format: "md", readOnly: false, guide: false };
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <CarryActions page={page} />, host);
  try {
    expect(host.textContent).toContain("Carry from previous day");
    const today = localDayKey();
    setCurrentDayKeyForTest(localDayKey(new Date(new Date().getFullYear(), new Date().getMonth(), new Date().getDate() + 1)));
    expect(localDayKey()).toBe(today);
    expect(host.textContent).toContain("Carry unfinished tasks → today");
  } finally { dispose(); }
});
