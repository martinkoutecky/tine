// One answer to "open the page named N" for every native E2E journey
// (AGENTS.md §5 "Page navigation in E2E has ONE implementation"; transcribed
// from master's scripts/lib/e2e-navigation.mjs for og).
//
// THE RULE THIS ENCODES. Quick Switcher rows are rendered by Solid's keyed
// `<For>` over a resource on a debounced query, so a keystroke can replace the
// rows. Never hold a WebDriver element handle across that list: find and
// activate in ONE `browser.execute` round trip, treat the routed page title as
// the readiness predicate, and retry the whole atomic selection until it holds.

const DEFAULT_TIMEOUT_MS = 20_000;

const nfc = (value) => (value ?? "").trim().normalize("NFC");

/** Use the same visible Journals navigation action as a user. */
export async function openJournals(browser) {
  const clicked = await browser.execute(() => {
    const item = [...document.querySelectorAll(".nav-item")]
      .find((element) => element.textContent?.trim() === "Journals");
    if (!(item instanceof HTMLElement)) return false;
    item.click();
    return true;
  });
  if (!clicked) throw new Error("Journals navigation item is absent");
  await browser.waitUntil(async () => (await browser.$$(".page-section")).length > 0, {
    timeout: 15_000, interval: 100, timeoutMsg: "Journals page did not render",
  });
}

/** The routed page title, or "" when no page is open. Never throws. */
export async function currentPageTitle(browser) {
  return nfc(await browser.execute(() => document.querySelector("h1.page-title")?.textContent ?? ""));
}

async function waitForTitle(browser, name, timeout, what) {
  await browser.waitUntil(async () => (await currentPageTitle(browser)) === nfc(name), {
    timeout, interval: 100, timeoutMsg: `${what}: the routed page title never became ${JSON.stringify(name)}`,
  });
}

/** Page rows (never block hits) the open switcher currently offers. */
export async function switcherPageRows(browser) {
  return browser.execute(() => [...document.querySelectorAll(".switcher-row:not(.block-result) .switcher-name")]
    .map((node) => (node.textContent ?? "").trim().normalize("NFC")));
}

async function openSwitcher(browser, opts, timeout) {
  const byButton = opts.entry === "button";
  for (let attempt = 1; ; attempt += 1) {
    if (byButton) {
      await browser.execute(() => {
        const button = document.querySelector('button[title^="Search (Ctrl+K)"]');
        if (button instanceof HTMLElement) button.click();
      });
    } else {
      await browser.keys(["Control", "k"]);
    }
    const input = await browser.$(".switcher-input");
    try {
      await input.waitForExist({ timeout: Math.max(1_500, Math.floor(timeout / 3)) });
      return input;
    } catch {
      if (attempt >= 3) throw new Error(`Quick Switcher did not open after ${attempt} attempts`);
    }
  }
}

/** Ensure the page named `name` is routed, through the Quick Switcher. Returns
 *  immediately when it already is. `opts.entry`: "shortcut" (Ctrl+K, default)
 *  or "button". `opts.query` searches another spelling (e.g. an alias) and
 *  activates the highest-ranked page row, still expecting canonical `name`. */
export async function openPageByName(browser, name, opts = {}) {
  const timeout = opts.timeout ?? DEFAULT_TIMEOUT_MS;
  if ((await currentPageTitle(browser)) === nfc(name)) return;
  const input = await openSwitcher(browser, opts, timeout);
  await input.setValue(opts.query ?? name);
  const selectOnce = () => browser.execute((wanted, firstPageResult) => {
    const target = wanted.trim().normalize("NFC");
    // Searching is shown for both the debounce and the outstanding read. A
    // retained row belongs to the previous query until this status disappears.
    if (document.querySelector('#switcher-results [role="status"]')) return false;
    const pages = [...document.querySelectorAll(".switcher-row")].filter((candidate) =>
      !candidate.classList.contains("block-result") && candidate.querySelector(".switcher-name"));
    const named = pages.find((candidate) => (candidate.querySelector(".switcher-name")?.textContent ?? "").trim().normalize("NFC") === target);
    const row = firstPageResult ? pages[0] : named;
    if (!row) return false;
    // Rows act on mousedown (QuickSwitcher.tsx): find and activate in one tick.
    row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  }, name, opts.query !== undefined);
  let offered = false;
  try {
    await browser.waitUntil(async () => {
      if ((await currentPageTitle(browser)) === nfc(name)) return true;
      offered = (await selectOnce()) || offered;
      return (await currentPageTitle(browser)) === nfc(name);
    }, { timeout, interval: 150, timeoutMsg: "timed out" });
  } catch {
    throw new Error(`Quick Switcher did not open ${JSON.stringify(name)} (${offered
      ? "a page row was activated, but the route never settled"
      : "no eligible page row was offered"}); offering: ${JSON.stringify(await switcherPageRows(browser))}`);
  }
  await waitForTitle(browser, name, timeout, "Quick Switcher");
  if (await browser.execute(() => Boolean(document.querySelector(".switcher-input")))) await browser.keys(["Escape"]);
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".switcher-input")), {
    timeout, interval: 100, timeoutMsg: "Quick Switcher remained open after the requested page became visible",
  });
}

/** Ensure `name` is routed by clicking a rendered `.page-ref` to it (tolerating
 *  `[[ ]]` decoration), retrying the atomic find+click until the title holds. */
export async function openPageByLink(browser, name, opts = {}) {
  const timeout = opts.timeout ?? DEFAULT_TIMEOUT_MS;
  if ((await currentPageTitle(browser)) === nfc(name)) return;
  const clickOnce = () => browser.execute((wanted) => {
    const target = wanted.trim().normalize("NFC");
    const undecorate = (text) => (text ?? "").trim().replace(/^\[\[/, "").replace(/\]\]$/, "").trim().normalize("NFC");
    const link = [...document.querySelectorAll(".page-ref")].find((node) => undecorate(node.textContent) === target);
    if (!link) return false;
    link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
    return true;
  }, name);
  let found = false;
  try {
    await browser.waitUntil(async () => {
      if ((await currentPageTitle(browser)) === nfc(name)) return true;
      found = (await clickOnce()) || found;
      return (await currentPageTitle(browser)) === nfc(name);
    }, { timeout, interval: 150, timeoutMsg: "timed out" });
  } catch {
    const refs = await browser.execute(() => [...document.querySelectorAll(".page-ref")].map((node) => node.textContent?.trim() ?? ""));
    throw new Error(`no rendered link routed to ${JSON.stringify(name)} (${found
      ? "a matching link was clicked but the route never settled" : "no .page-ref matched"}); links: ${JSON.stringify(refs.slice(0, 40))}`);
  }
}
