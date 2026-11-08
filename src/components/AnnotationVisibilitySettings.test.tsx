import { afterEach, expect, it } from "vitest";
import { render } from "solid-js/web";
import { AnnotationVisibilitySettings } from "./AnnotationVisibilitySettings";
import { authorMarksHidden, setAuthorMarksHidden } from "../authorMarksPref";
import { commentMarginHidden, forceCommentMarginShown } from "../commentMarginPref";
import { setActiveLocale } from "../i18n";

afterEach(async () => { setAuthorMarksHidden(false); forceCommentMarginShown(); await setActiveLocale("en"); document.body.innerHTML = ""; });
it.each(["en", "zh-CN"])("%s Settings groups two independently labelled switches", async locale => {
  await setActiveLocale(locale); setAuthorMarksHidden(false); forceCommentMarginShown();
  const root = document.createElement("div"); document.body.append(root);
  const dispose = render(() => <AnnotationVisibilitySettings />, root);
  try {
    const marks = root.querySelector<HTMLButtonElement>('[data-setting-id="settings-author-marks"] [role="switch"]')!;
    const comments = root.querySelector<HTMLButtonElement>('[data-setting-id="settings-ui-comment-margin"] [role="switch"]')!;
    expect(marks.closest('[role="group"]')?.contains(comments)).toBe(true);
    expect(root.textContent).toContain(locale === "en" ? "Show/Hide author marks" : "显示/隐藏作者标记");
    marks.click(); expect(authorMarksHidden()).toBe(true); expect(commentMarginHidden()).toBe(false);
    comments.click(); expect(authorMarksHidden()).toBe(true); expect(commentMarginHidden()).toBe(true);
    marks.click(); expect(authorMarksHidden()).toBe(false); expect(commentMarginHidden()).toBe(true);
  } finally { dispose(); }
});
