// Resolve the saved light/dark preference before first paint (GH #401).
// A classic same-origin script, not inline: the app CSP allows only 'self'
// scripts (I-22, src/csp.guard.test.ts). It lives under assets/ because the
// live export, CLI and demo ship only index.html + assets/ of the app bundle.
try {
  const preference = localStorage.getItem("logseq-claude.theme");
  const dark = preference === "dark" ||
    (preference === "system" && matchMedia("(prefers-color-scheme: dark)").matches);
  document.documentElement.setAttribute("data-theme", dark ? "dark" : "light");
} catch (_) {
  // The deterministic light default on <html> remains usable.
}
