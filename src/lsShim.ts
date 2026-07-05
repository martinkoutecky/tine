import lsShimCss from "./styles/ls-shim.css?inline";

export const LS_SHIM_STYLE_ID = "tine-ls-shim";
export const CUSTOM_CSS_STYLE_ID = "tine-custom-css";
export const LS_SHIM_CSS = lsShimCss;

export function ensureLsShimStyle(): HTMLStyleElement | null {
  if (typeof document === "undefined") return null;

  let el = document.getElementById(LS_SHIM_STYLE_ID) as HTMLStyleElement | null;
  if (el && el.tagName !== "STYLE") {
    el.remove();
    el = null;
  }
  if (!el) {
    el = document.createElement("style");
    el.id = LS_SHIM_STYLE_ID;
    el.textContent = LS_SHIM_CSS;
    const custom = document.getElementById(CUSTOM_CSS_STYLE_ID);
    if (custom?.parentNode === document.head) document.head.insertBefore(el, custom);
    else document.head.appendChild(el);
  } else if (el.textContent !== LS_SHIM_CSS) {
    el.textContent = LS_SHIM_CSS;
  }

  const custom = document.getElementById(CUSTOM_CSS_STYLE_ID);
  if (custom?.parentNode === document.head) {
    const headNodes = Array.from(document.head.childNodes);
    if (headNodes.indexOf(el) > headNodes.indexOf(custom)) {
      document.head.insertBefore(el, custom);
    }
  }

  return el;
}
