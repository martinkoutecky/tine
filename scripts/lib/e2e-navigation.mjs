// One answer to "open the page named N" for every native E2E journey.
//
// WHY THIS EXISTS (flake census, 2026-09-04). Sixteen `scripts/e2e-*.mjs` files
// each carried their own navigate/openPage/gotoPage helper, with nine distinct
// switcher-row selector strings, timeouts from 5s to 60s, NFC normalization in
// three of them, and three different ways of activating a row. A readiness bug
// found in one of those copies was therefore never fixed in the other fifteen,
// which is the actual reason E2E flakes keep costing whole release cycles: the
// fix has no single place to land.
//
// THE FLAKE THIS REMOVES. Quick Switcher results come from a `createResource`
// keyed on a debounced query, rendered through `<For each={section.items}>`
// (src/components/QuickSwitcher.tsx). Solid's `For` is keyed by reference, so
// when the resource resolves it REPLACES the row elements. WebDriver's
// `setValue` types character by character, and each keystroke can settle its own
// query — so a harness that does `$(row).waitForExist()` and then `row.click()`
// can capture a row from an intermediate result set and click a node that has
// since been detached. Nothing happens, and the journey fails 10 seconds later
// on the page title, not on the row. `e2e-right-sidebar-collapse.mjs` is the
// type specimen precisely because it opens "Page A" then "Page B": the shared
// prefix means an intermediate result set already contains a row whose name
// matches exactly, so the early capture looks correct.
//
// THE RULE THIS ENCODES. Never hold an element handle across a list that
// re-renders. Find and activate in ONE `browser.execute` round trip, treat the
// routed page title as the readiness predicate, and retry the whole atomic
// selection until that predicate holds. Retrying is safe because selecting a
// row is idempotent.
//
// It also fixes, once, three things that were right in some copies and wrong in
// others: block hits are excluded (a block whose text contains the page title
// outranked the page itself and broke the hosted Windows managed-storage
// journey), names are compared NFC-normalized (macOS/Linux filename twins), and
// `[[ ]]` link decoration is tolerated when routing through a rendered link,
// since `:ui/show-brackets?` defaults to on.

const DEFAULT_TIMEOUT_MS = 20_000;

const nfc = (value) => (value ?? "").trim().normalize("NFC");

/**
 * The routed page title, or "" when no page is open. Never throws.
 *
 * `pane` scopes the question to one split pane (`data-pane-id`), which is the
 * only correct readiness predicate for a journey that routes panes
 * independently: the window-level `h1.page-title` answers for whichever pane
 * happens to be first in the DOM.
 */
export async function currentPageTitle(browser, pane) {
  const text = await browser.execute(
    (paneId) =>
      (paneId
        ? document.querySelector(`[data-pane-id="${paneId}"] .page-title`)
        : document.querySelector("h1.page-title")
      )?.textContent ?? "",
    pane ?? null,
  );
  return nfc(text);
}

async function waitForTitle(browser, name, timeoutMs, what, pane) {
  await browser.waitUntil(async () => (await currentPageTitle(browser, pane)) === nfc(name), {
    timeout: timeoutMs,
    interval: 100,
    timeoutMsg: `${what}: the routed page title never became ${JSON.stringify(name)}`,
  });
}

/** What the switcher is currently offering — for failure messages that say why. */
async function switcherOffers(browser) {
  return browser.execute(() =>
    [...document.querySelectorAll(".switcher-row")].map((row) => ({
      name: row.querySelector(".switcher-name")?.textContent?.trim() ?? "",
      kind: row.querySelector(".switcher-kind")?.textContent?.trim() ?? "",
      block: row.classList.contains("block-result"),
    })),
  );
}

/**
 * Open a page through the Quick Switcher — the production keyboard route, and
 * the one that works when the sidebar is collapsed, absent, or outside a narrow
 * viewport.
 *
 * Returns immediately if `name` is already the routed page. That makes this
 * "ensure this page is open", not "perform a navigation" — a journey asserting
 * on history or recents must not use it to re-open the page it is already on,
 * because no navigation would occur. No journey does today.
 *
 * `opts.entry` picks how the switcher is opened: "shortcut" (default, Ctrl+K)
 * or "button", which clicks the visible search control instead. Windows needs
 * "button" because attaching WebView2 does not guarantee native keyboard focus,
 * and fixture navigation is not a shortcut assertion.
 *
 * `opts.pane` scopes the readiness predicate to one split pane; the caller is
 * responsible for focusing that pane first, since which pane a switcher opens
 * into is the product behaviour, not the harness's to arrange.
 */
export async function openPageByName(browser, name, opts = {}) {
  const timeout = opts.timeout ?? DEFAULT_TIMEOUT_MS;
  const pane = opts.pane;
  if ((await currentPageTitle(browser, pane)) === nfc(name)) return;

  if ((opts.entry ?? "shortcut") === "button") {
    const search = await browser.$('button[title^="Search (Ctrl+K)"]');
    await search.waitForClickable({ timeout, timeoutMsg: "the Search control was never clickable" });
    await search.click();
  } else {
    await browser.keys(["Control", "k"]);
  }
  const input = await browser.$(".switcher-input");
  await input.waitForExist({ timeout, timeoutMsg: "Quick Switcher did not open" });
  await input.setValue(name);

  // One atomic attempt: find the exact page row and activate it in a single
  // round trip, so no handle outlives the re-render. Reported so a failure can
  // say whether the row was never offered or was offered and did not route.
  const selectOnce = () =>
    browser.execute((wanted) => {
      const target = wanted.trim().normalize("NFC");
      const row = [...document.querySelectorAll(".switcher-row")].find(
        (candidate) =>
          !candidate.classList.contains("block-result") &&
          (candidate.querySelector(".switcher-name")?.textContent ?? "").trim().normalize("NFC") === target,
      );
      if (!row) return false;
      // The row acts on mousedown (QuickSwitcher.tsx), which is also what a real
      // pointer delivers first; dispatching it here keeps find and activate in
      // the same tick.
      row.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true, button: 0 }));
      return true;
    }, name);

  let offered = false;
  try {
    await browser.waitUntil(
      async () => {
        if ((await currentPageTitle(browser, pane)) === nfc(name)) return true;
        offered = (await selectOnce()) || offered;
        return (await currentPageTitle(browser, pane)) === nfc(name);
      },
      { timeout, interval: 150, timeoutMsg: "timed out" },
    );
  } catch {
    const offers = await switcherOffers(browser);
    throw new Error(
      `Quick Switcher did not open ${JSON.stringify(name)} ` +
        `(${offered ? "the exact page row was offered and activated, but the route never settled" : "no non-block row with that exact name was ever offered"}); ` +
        `offering: ${JSON.stringify(offers)}`,
    );
  }

  await waitForTitle(browser, name, timeout, "Quick Switcher", pane);
  // Startup can restore this route while our switcher is still searching.
  // A matching title underneath the modal is not a completed navigation:
  // dismiss the switcher we opened before handing control to the next action.
  if (await browser.execute(() => Boolean(document.querySelector(".switcher-input")))) {
    await browser.keys(["Escape"]);
  }
  await browser.waitUntil(() => browser.execute(() => !document.querySelector(".switcher-input")), {
    timeout,
    interval: 100,
    timeoutMsg: "Quick Switcher remained open after the requested page became visible",
  });

}

/**
 * Open a page by clicking a rendered link to it — the route a journey wants
 * when the link itself, not the switcher, is the thing under test.
 *
 * `[[ ]]` decoration inside a `.page-ref` is tolerated: `:ui/show-brackets?`
 * defaults to on (src/render/inline.tsx, pinned by src/render/showBrackets.test.tsx),
 * so the anchor's visible text is not the bare title. The contract is "a link to
 * this page is reachable", never how the link is decorated.
 */
export async function openPageByLink(browser, name, opts = {}) {
  const timeout = opts.timeout ?? DEFAULT_TIMEOUT_MS;
  if ((await currentPageTitle(browser)) === nfc(name)) return;

  const clickOnce = () =>
    browser.execute((wanted) => {
      const target = wanted.trim().normalize("NFC");
      const undecorate = (text) =>
        (text ?? "").trim().replace(/^\[\[/, "").replace(/\]\]$/, "").trim().normalize("NFC");
      const link = [...document.querySelectorAll(".page-ref")].find((node) => undecorate(node.textContent) === target);
      if (!link) return false;
      link.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, button: 0 }));
      return true;
    }, name);

  let found = false;
  try {
    await browser.waitUntil(
      async () => {
        if ((await currentPageTitle(browser)) === nfc(name)) return true;
        found = (await clickOnce()) || found;
        return (await currentPageTitle(browser)) === nfc(name);
      },
      { timeout, interval: 150, timeoutMsg: "timed out" },
    );
  } catch {
    const refs = await browser.execute(() =>
      [...document.querySelectorAll(".page-ref")].map((node) => node.textContent?.trim() ?? ""),
    );
    throw new Error(
      `no rendered link routed to ${JSON.stringify(name)} ` +
        `(${found ? "a matching link was clicked but the route never settled" : "no .page-ref matched that name"}); ` +
        `links present: ${JSON.stringify(refs.slice(0, 40))}`,
    );
  }

  await waitForTitle(browser, name, timeout, "page link");
}
