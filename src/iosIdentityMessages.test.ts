import { expect, it } from "vitest";
import { IDENTITIES } from "../scripts/lib/app-identity.mjs";
import { setActiveLocale, t } from "./i18n";

it("iOS Welcome and folder refusals name the selected app's own folders in both locales", async () => {
  try {
    for (const locale of ["en", "zh-CN"]) {
      await setActiveLocale(locale);
      for (const identity of Object.values(IDENTITIES)) {
        const app = identity.ios.displayName;
        for (const text of [
          t("welcome-on-iphone-and-ipad-graphs-can-live", { app }),
          t("graph-tineoutline-can-only-open-folders-inside-on", { app }),
          t("graph-choose-a-folder-inside-on-my-iphone", { app }),
          t("graph-tineoutline-can-only-create-graphs-inside-on", { app }),
        ]) {
          expect(text).toContain(`→ ${app}`);
          expect(text).toContain(`iCloud Drive → ${app}`);
        }
      }
    }
  } finally {
    await setActiveLocale("en");
  }
});
