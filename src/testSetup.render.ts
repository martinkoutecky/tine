import { beforeEach, vi } from "vitest";
import { delegateEvents, DelegatedEvents } from "solid-js/web";
import "./testSetup.range";

// The ordinary render suite explicitly covers classic; vitest.cm6.config.ts
// runs the same editor-neutral driver coverage with CM6 forced on.
// Stay lazy: parser-only tests deliberately supply a minimal wasm mock and
// must not import the application backend just because this setup runs.
const classic = vi.hoisted(() => ({ choose: undefined as (() => Promise<void>) | undefined }));
vi.mock("./editor/cm6Setting", async importOriginal => {
  const setting = await importOriginal<typeof import("./editor/cm6Setting")>();
  const { preferenceReadCurrent, preferenceRevision } = await import("./preferenceWrites");
  classic.choose = async () => {
    if (setting.cm6EditorEnabled()) setting.setCm6EditorEnabled(false);
    await vi.waitFor(() => {
      if (!preferenceReadCurrent(setting.cm6EditorEnabled, preferenceRevision(setting.cm6EditorEnabled))) throw new Error("classic choice is still being saved");
    });
  };
  await classic.choose();
  return setting;
});
beforeEach(async () => {
  await classic.choose?.();
  delegateEvents([...DelegatedEvents], document);
  // jsdom does not implement this browser method. Install the same no-op
  // boundary before every test because focused viewer tests may replace and
  // remove it while cleaning up their own spies.
  if (typeof HTMLElement.prototype.scrollIntoView !== "function") {
    HTMLElement.prototype.scrollIntoView = () => {};
  }
});
