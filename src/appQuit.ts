import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./backend";
import { msg } from "./i18n";
import { pushToast } from "./toasts";

/** Request the tray's ordinary guarded close in every graph window. Never exit
 * from the keyboard handler: each window must flush before host retirement. */
export function requestAppQuit(): void {
  if (!isTauri()) return;
  void invoke("request_app_quit").catch(() => {
    pushToast(msg("commands-couldn-t-request-quit"), "error");
  });
}
