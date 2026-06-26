// Detect — and loudly surface — when Tine is painting on the CPU. Speed is the
// whole pitch; a silent software-rendering fallback (e.g. an AppImage whose
// bundled GL stack doesn't match the host driver, or WebKitGTK's DMABUF/EGL init
// failing) makes scrolling feel sluggish for no obvious reason. Better to say so
// than to let the user think Tine is just slow.
//
// How we detect it: WebKit's 2D compositing and WebGL share the same GBM/EGL/GL
// stack, so if that stack falls back to Mesa's software rasterizer, the WebGL
// renderer string reports it (`llvmpipe`, `swrast`, …). That string is a reliable
// proxy for "the GPU path isn't engaging, so compositing is on the CPU too".

import { backend, isTauri } from "./backend";
import { pushToast } from "./ui";

// Mesa/ANGLE software-rasterizer signatures. `basic render` catches Windows'
// "Microsoft Basic Render Driver" (WARP) under RDP/VMs.
const SOFTWARE_RE = /llvmpipe|softpipe|swrast|software|swiftshader|basic render/i;

/** The unmasked GL renderer string, or null if it can't be determined (some
 *  hardened WebKit builds hide it — in which case we stay quiet rather than risk
 *  a false alarm). */
function glRenderer(): string | null {
  try {
    const canvas = document.createElement("canvas");
    const gl = (canvas.getContext("webgl2") ||
      canvas.getContext("webgl") ||
      canvas.getContext("experimental-webgl")) as WebGLRenderingContext | null;
    if (!gl) return null;
    const ext = gl.getExtension("WEBGL_debug_renderer_info");
    const raw = ext
      ? gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)
      : gl.getParameter(gl.RENDERER);
    return typeof raw === "string" && raw.trim() ? raw : null;
  } catch {
    return null;
  }
}

/** True only when we're confident the GL stack is the software rasterizer.
 *  Unknown renderer → false (don't cry wolf). Exported for testing. */
export function isSoftwareRenderer(renderer: string | null): boolean {
  return renderer != null && SOFTWARE_RE.test(renderer);
}

/** Probe once on startup; if Tine is rendering on the CPU, show a sticky toast
 *  explaining why and (when relevant) how to get the fast path back. Tauri-only:
 *  in the browser/screenshot mock the renderer is often headless software, which
 *  would be a false positive — and the warning is meaningless there anyway. */
export async function warnIfSoftwareRendering(): Promise<void> {
  if (!isTauri()) return;
  try {
    if (!isSoftwareRenderer(glRenderer())) return;

    // Software confirmed from the webview. Ask the backend why, to tailor the
    // message (was it forced on purpose? are we in an AppImage?).
    const env = await backend()
      .gpuEnv()
      .catch(() => ({ software_forced: false, appimage: false }));

    if (env.software_forced) {
      // The user (or our TINE_GPU=0 escape hatch) turned GPU compositing off —
      // expected, but still worth a reminder that it costs scroll smoothness.
      pushToast(
        "Software rendering is on (TINE_GPU=0 / WEBKIT_DISABLE_DMABUF_RENDERER) — scrolling may feel slow. Unset it to use your GPU.",
        "info",
        { sticky: true }
      );
    } else if (env.appimage) {
      // The surprise case: GPU was requested but the AppImage's bundled graphics
      // libraries don't match the host driver, so WebKit fell back to the CPU.
      pushToast(
        "This AppImage is rendering on the CPU (its bundled graphics libraries don't match your system), so scrolling may feel slow. The .deb / .rpm packages use your system's GPU drivers and are noticeably faster.",
        "warn",
        { sticky: true }
      );
    } else {
      pushToast(
        "Tine is rendering on the CPU — your GPU's accelerated path is unavailable, so scrolling may feel slow. Check that your graphics drivers / hardware acceleration are working.",
        "warn",
        { sticky: true }
      );
    }
  } catch {
    /* best-effort: a detection hiccup must never block startup */
  }
}
