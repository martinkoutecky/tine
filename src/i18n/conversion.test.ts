import { expect, it } from "vitest";
import { msg, renderEnglish, t } from "./index";
import { pushReferenceChangeNotice, setToasts, toasts } from "../toasts";

it("preserves whole English count messages at zero, one, many and ungrouped large counts", () => {
  expect(t("carry-result", { count: 0 })).toBe("No unfinished tasks to carry");
  expect(t("carry-result", { count: 1 })).toBe("Carried 1 item to today");
  expect(t("carry-result", { count: 2400 })).toBe("Carried 2400 items to today");
  expect(t("filedrop-inserted", { count: 2 })).toBe("Inserted 2 files");
  expect(t("inpage-replace-result", { count: 1, skipped: 0 })).toBe("Replaced 1 match.");
  expect(t("inpage-replace-result", { count: 2, skipped: 3 })).toBe("Replaced 2 matches; 3 inside links, markup or read-only blocks left as they were.");
  expect(t("failure-boundary-slow-operations", { count: 1, seconds: "35" })).toContain("an operation that has been running for over 35s.");
  expect(t("failure-boundary-slow-operations", { count: 2, seconds: "35" })).toContain("2 operations that have been running for over 35s.");
});

it("stores reference-change notices as messages and preserves English reporting", () => {
  setToasts([]);
  expect(pushReferenceChangeNotice(0, false, () => {})).toBeNull();
  pushReferenceChangeNotice(1, false, () => {});
  pushReferenceChangeNotice(null, true, () => {});
  expect(toasts().map(toast => toast.text.kind)).toEqual(["msg", "msg"]);
  expect(toasts().map(toast => toast.message)).toEqual([
    "1 reference is now broken", "References may now point to this block",
  ]);
  expect(renderEnglish(msg("toasts-reference-change", { count: 1200, known: 1, transferred: 1 })))
    .toBe("1200 references now point to this block");
  setToasts([]);
});
