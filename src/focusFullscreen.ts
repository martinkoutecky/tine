/** Focus-mode ownership of native fullscreen. Requests queue; a superseded
 * request waiting at a generation check resolves without native mutation. A
 * native call already in flight may finish before a newer request reverses it.
 * Exit preserves fullscreen that was on before focus mode. Non-Tauri calls
 * resolve without native work. Native errors reject without retry or rollback;
 * the UI focus signal is separate. O(1) native calls per request. */
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
  tail = task.then(() => undefined, () => undefined);
  return task;
}
