/** Wait for a modal drawer before native interaction. One bounded DOM observation
 * per poll, no graph IO; failure uses the journey's existing eight-second bound. */
export async function waitDrawer(browser, side) {
  await browser.waitUntil(() => browser.execute((wanted) => {
    const panels = document.querySelectorAll("[data-mobile-drawer-panel]");
    const panel = panels[0];
    // Active state precedes the slide. Native clicks need resting geometry.
    return document.querySelector(".app-container")?.getAttribute("data-active-drawer") === wanted
      && document.querySelectorAll(".mobile-drawer-scrim").length === 1
      && panels.length === 1 && panel.getAnimations().length === 0
      && (getComputedStyle(panel).transform === "none" || new DOMMatrixReadOnly(getComputedStyle(panel).transform).m41 === 0);
  },
  side), { timeout: 8_000, timeoutMsg: `${side} drawer did not settle before native interaction` });
}

/** Since #1005 the desktop window can be 390 px wide, below the 640 px drawer
 * breakpoint (Logseq's `sm`): an open left sidebar then becomes a modal drawer
 * over the page, as in Logseq. Close it the way a user would (Escape). Call after
 * narrowing the window; a few DOM polls, no graph IO. */
export async function closeNarrowDrawer(browser) {
  await browser.waitUntil(() => browser.execute(() => innerWidth < 640), { timeout: 5_000, timeoutMsg: "window did not narrow" });
  await new Promise((resolve) => setTimeout(resolve, 300));
  const open = await browser.execute(() => Boolean(document.querySelector(".app-container")?.getAttribute("data-active-drawer")));
  if (!open) return;
  await browser.keys("Escape");
  await browser.waitUntil(() => browser.execute(() =>
    !document.querySelector(".app-container")?.getAttribute("data-active-drawer")
    && document.querySelectorAll(".mobile-drawer-scrim").length === 0),
  { timeout: 8_000, timeoutMsg: "narrow-window drawer did not close" });
}
