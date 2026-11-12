/** Dismiss setup notices through their controls before exercising another surface.
 * Scratch app-data and owner-debug fixtures can produce persistent notifications
 * over the journey's pointer targets. Keep those fixtures and their evidence.
 */
export async function dismissToasts(browser) {
  await browser.execute(() => {
    for (const close of document.querySelectorAll(".toast-stack .toast-close")) close.click();
  });
}
