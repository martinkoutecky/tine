// Shared real Settings/native-confirmation activation used by Managed journeys.
import { execFileSync } from "node:child_process";

export async function enableManagedStorage(browser, { onPhase = () => {} } = {}) {
  const xdo = (...args) => execFileSync(process.env.E2E_XDOTOOL || "xdotool", args, {
    encoding: "utf8",
    env: process.env.E2E_XDOTOOL_LIB
      ? { ...process.env, LD_LIBRARY_PATH: process.env.E2E_XDOTOOL_LIB }
      : process.env,
  }).trim();
  const windowIds = () => {
    try { return xdo("search", "--onlyvisible", "--name", "^Tine$").split(/\s+/).filter(Boolean); }
    catch { return []; }
  };
  const waitFor = async (predicate, timeout, timeoutMsg) => {
    let result;
    await browser.waitUntil(async () => Boolean(result = await predicate()), { timeout, interval: 100, timeoutMsg });
    return result;
  };
  const visibleButtonContaining = async (text) => {
    for (const button of await browser.$$("button")) {
      if (await button.isDisplayed() && (await button.getText()).includes(text)) return button;
    }
  };
  onPhase("open-settings");
  const trigger = await browser.$('button[title^="Settings"]');
  await trigger.waitForDisplayed({ timeout: 30_000 });
  await trigger.click();
  await browser.$(".settings-modal").waitForDisplayed({ timeout: 30_000 });
  const tab = await waitFor(
    () => visibleButtonContaining("Backups & recovery"),
    30_000,
    "Backups & recovery settings tab was absent",
  );
  await tab.click();
  const experimental = await browser.$(".settings-experimental .settings-advanced-toggle");
  await experimental.waitForDisplayed({ timeout: 30_000 });
  if ((await experimental.getAttribute("aria-expanded")) !== "true") await experimental.click();
  onPhase("find-enable-action");
  const action = await waitFor(
    () => visibleButtonContaining("Enable Tine-managed storage..."),
    30_000,
    "managed activation action was absent",
  );
  const before = new Set(windowIds("^Tine$"));
  onPhase("click-enable-action");
  await action.click();
  onPhase("confirmation-dialog");
  const dialog = await waitFor(() => windowIds().find(id => !before.has(id)), 30_000,
    "managed activation did not show its native confirmation");
  xdo("windowactivate", "--sync", dialog);
  xdo("key", "--clearmodifiers", "alt+y");
  onPhase("confirmation-dismissal");
  await waitFor(() => !windowIds().includes(dialog), 30_000,
    "managed activation confirmation did not close");
  onPhase("active-status");
  await browser.waitUntil(async () => (await browser.$("body").getText()).includes("Tine-managed storage active"), {
    timeout: 300_000,
    interval: 250,
    timeoutMsg: "managed activation did not reach active",
  });
  onPhase("close-settings");
  const close = await browser.$(".settings-pane-head .icon-btn:not(.settings-maximize)");
  await close.click();
  await browser.$(".settings-modal").waitForExist({ reverse: true, timeout: 30_000 });
  onPhase("complete");
}

