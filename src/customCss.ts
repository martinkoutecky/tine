/** The window's one owner of the graph's `logseq/custom.css` stylesheet (GH #610).
 *
 *  `applyCustomCss` paints the file's text into the `#tine-custom-css` style
 *  element (after the Logseq shim and the theme gallery, so it wins ties).
 *  "Disable custom CSS" is a session-only safe mode: it blanks the element but
 *  keeps the text, so enabling it again repaints without re-reading the file,
 *  and an external edit while disabled is remembered, not shown. Nothing here
 *  is persisted: a restart always starts with custom CSS on, so a broken
 *  stylesheet can always be recovered from Settings or by editing the file.
 *  The style knobs layer (`#tine-knobs`, src/styleKnobs.ts) is silenced by the
 *  same switch. */
import { createSignal } from "solid-js";
import { CUSTOM_CSS_STYLE_ID, ensureLsShimStyle } from "./lsShim";
import { setKnobsSafeMode } from "./styleKnobs";
import { ensureThemeStyle } from "./themeGallery";

let lastCss = "";
let globalCss = "";
let graphVersion = 0;
let globalVersion = 0;
/** A successful save or paint retires reads started against the previous CSS. */
export const customCssVersion = (global = false): number => global ? globalVersion : graphVersion;
const [disabled, setDisabled] = createSignal(false);

/** Whether custom CSS is switched off for this session (safe mode). */
export const customCssDisabled = disabled;

function paint(): void {
  if (typeof document === "undefined") return;
  ensureLsShimStyle();
  ensureThemeStyle();
  let global = document.getElementById("tine-global-css");
  if (!global) {
    global = document.createElement("style");
    global.id = "tine-global-css";
  }
  global.textContent = disabled() ? "" : globalCss;
  document.head.appendChild(global);
  let el = document.getElementById(CUSTOM_CSS_STYLE_ID);
  if (!el) {
    el = document.createElement("style");
    el.id = CUSTOM_CSS_STYLE_ID;
  }
  el.textContent = disabled() ? "" : lastCss;
  document.head.appendChild(el);
}

/** Remember and show the graph's current custom.css text. */
export function applyCustomCss(css: string): void {
  ++graphVersion;
  lastCss = css;
  paint();
}

/** Device CSS precedes graph CSS, so graphs can override the shared defaults. */
export function applyGlobalCss(css: string): void {
  ++globalVersion;
  globalCss = css;
  paint();
}

/** Switch custom CSS off (or back on) for this session only. */
export function setCustomCssDisabled(value: boolean): void {
  setDisabled(value);
  setKnobsSafeMode(value); // K4: the style knobs share this safe mode
  paint();
}
