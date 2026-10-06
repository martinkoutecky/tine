import { createSignal, onCleanup, onMount, Show, type JSX } from "solid-js";
import { backend } from "../backend";
import { pushToast } from "../toasts";
import type { TrayStatus } from "../backendTypes";
import { writePreference, loadPreference } from "../preferenceWrites";
import {
  KEY_TRAY_MINIMIZE,
  KEY_TRAY_SHOW,
  KEY_TRAY_START_MINIMIZED,
  trayDependentControlsEnabled,
  applyTray,
  persistTrayPreference,
  trayNote,
} from "../tray";
import { Field, Toggle } from "./settingsField";

/** Desktop system-tray settings (GH #625): three device toggles, all off by
 *  default. Rendered only where the native side reports a tray (never on
 *  Android/iOS or in the mock). Every change persists the key and then asks
 *  the native side to create/remove the icon, so no restart is needed. */
export function TraySettings(): JSX.Element {
  const [status, setStatus] = createSignal<TrayStatus | null>(null);
  const [show, setShow] = createSignal(false);
  const [minimize, setMinimize] = createSignal(false);
  const [start, setStart] = createSignal(false);
  let alive = true;
  onCleanup(() => { alive = false; });

  let loaded = false;
  const load = () => {
    if (loaded) return;
    loaded = true;
    toggleShow = bind(show, setShow, KEY_TRAY_SHOW, "tray icon preference");
    toggleMinimize = bind(minimize, setMinimize, KEY_TRAY_MINIMIZE, "minimize-to-tray preference");
    toggleStart = bind(start, setStart, KEY_TRAY_START_MINIMIZED, "start-minimized preference");
  };
  const publish = (next: TrayStatus | void) => {
    if (!alive || !next) return;
    setStatus(next);
    if (next.supported) load();
  };
  const bind = (read: () => boolean, set: (value: boolean) => void, key: string, label: string) => {
    loadPreference(read, set, () => backend().getAppBool(key, false), (value) => value, label, () => alive);
    return () => writePreference(read, set, !read(), async (next) => publish(await persistTrayPreference(key, next)), label);
  };
  let toggleShow = () => {};
  let toggleMinimize = () => {};
  let toggleStart = () => {};
  onMount(() => {
    applyTray().then(publish).catch(() => { if (alive) pushToast("Could not read the system tray state.", "error"); });
  });

  return (
    <Show when={status()?.supported}>
      <Field
        label="Show Tine in the system tray"
        hint="Adds a tray / notification-area (macOS: menu-bar) icon. Click it to show or hide the main window; its menu has Open Tine, Quick Capture and Quit. Closing the window still quits Tine. Off by default."
      >
        <Toggle on={show()} onClick={() => toggleShow()} />
      </Field>
      <Field
        label="Minimize to tray"
        hint="Minimizing the main window hides it from the taskbar or dock; restore it from the tray icon. Needs “Show Tine in the system tray”."
      >
        <Toggle on={minimize()} disabled={!trayDependentControlsEnabled(show())} onClick={() => toggleMinimize()} />
      </Field>
      <Field
        label="Start minimized to tray"
        hint="Launch with the main window hidden; open it from the tray icon. Launching Tine again, or opening a graph or link, always shows it. Needs “Show Tine in the system tray”."
      >
        <Toggle on={start()} disabled={!trayDependentControlsEnabled(show())} onClick={() => toggleStart()} />
      </Field>
      <Show when={trayNote(show(), status())}>
        {(note) => <div class="settings-hint settings-field-hint" role="status" data-tray-note>{note()}</div>}
      </Show>
    </Show>
  );
}
