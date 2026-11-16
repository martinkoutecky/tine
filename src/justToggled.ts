/** Theme hook (GH discussion #714): a short-lived class on the task checkbox and
 *  marker chip of a task the USER just toggled, so theme CSS can play a
 *  keyframe `animation` on a toggle and not whenever the element is drawn.
 *
 *  - Added by a pointer toggle only: the outline checkbox, the marker chip, and
 *    the checkbox of a task shown inside a block reference or query result. Never
 *    on page load, never when a virtualized row remounts, and never for a change
 *    that arrives from sync or an external editor (the class is put on the
 *    elements the user pressed, not derived from the task's state, so a remounted
 *    row cannot carry it).
 *  - Removed after `JUST_TOGGLED_MS`, a window longer than any calm animation;
 *    a second toggle inside the window restarts it.
 *
 *  Documented for theme authors in the Guide (Appearance, custom CSS). */
export const JUST_TOGGLED_CLASS = "just-toggled";
export const JUST_TOGGLED_MS = 700;

const timers = new WeakMap<Element, ReturnType<typeof setTimeout>>();

/** Mark `el` as just toggled for `JUST_TOGGLED_MS`. */
export function flashJustToggled(el: Element): void {
  const running = timers.get(el);
  if (running !== undefined) {
    clearTimeout(running);
    el.classList.remove(JUST_TOGGLED_CLASS);
    void (el as HTMLElement).offsetWidth; // restart a running keyframe animation
  }
  el.classList.add(JUST_TOGGLED_CLASS);
  timers.set(el, setTimeout(() => {
    timers.delete(el);
    el.classList.remove(JUST_TOGGLED_CLASS);
  }, JUST_TOGGLED_MS));
}

/** Mark the task checkbox and marker chip next to `from` (one of them, pressed
 *  by the user) as just toggled. */
export function flashTaskToggle(from: Element): void {
  flashJustToggled(from);
  for (const sibling of from.parentElement?.children ?? []) {
    if (sibling !== from && (sibling.classList.contains("block-task-checkbox") || sibling.classList.contains("block-marker"))) {
      flashJustToggled(sibling);
    }
  }
}
