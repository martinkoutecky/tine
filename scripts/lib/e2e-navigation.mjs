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
