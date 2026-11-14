// Select an engine through the shipped setting; both native engine suites use
// this boundary so the opt-out switch has one interpretation in the harness.
export async function chooseEditor(browser, kind) {
  await browser.$('button[title^="Settings"]').click();
  await browser.$(".settings-modal").waitForDisplayed();
  await browser.$(".settings-search-input").setValue("CodeMirror");
  const result = await browser.$('[data-setting-target="settings-cm6-label"]');
  await result.waitForDisplayed();
  await result.click();
  const toggle = await browser.$('[data-setting-id="settings-cm6-label"] [role="switch"]');
  await toggle.waitForDisplayed();
  const classic = String(kind === "textarea");
  if (await toggle.getAttribute("aria-checked") !== classic) await toggle.click();
  await browser.waitUntil(async () => await toggle.getAttribute("aria-checked") === classic);
  await browser.keys("Escape");
}
