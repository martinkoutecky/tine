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
