/** Native fullscreen ownership for focus mode.
 * setFocusFullscreen: O(1) native calls, serialized. A newer request cancels a
 * pending older intent. Native errors reject; the UI keeps its focus signal.
 * Callers need not track the window's previous fullscreen state. */
import { isTauri } from "./backend";

let generation = 0;
let requestedActive = false;
let ownsFullscreen = false;
let tail: Promise<void> = Promise.resolve();

async function appWindow() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  return getCurrentWindow();
}

export function setFocusFullscreen(active: boolean): Promise<void> {
  requestedActive = active;
  const request = ++generation;
  if (!isTauri()) return Promise.resolve();
  const task = tail.then(async () => {
    if (request !== generation || requestedActive !== active) return;
    if (active) {
      const window = await appWindow();
      if (request !== generation || !requestedActive) return;
      const wasFullscreen = await window.isFullscreen();
      if (request !== generation || !requestedActive) return;
      if (!wasFullscreen) {
        await window.setFullscreen(true);
        ownsFullscreen = true;
      }
    } else if (ownsFullscreen) {
      const window = await appWindow();
      if (request !== generation || requestedActive) return;
      await window.setFullscreen(false);
      ownsFullscreen = false;
    }
  });
  tail = task.catch(() => {});
  return task;
}
