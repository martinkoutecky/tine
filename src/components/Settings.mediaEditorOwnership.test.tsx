import { afterEach, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { backend } from "../backend";
import { MEDIA_EDITORS } from "../mediaEditors";
import { mediaEditorCommand, setMediaEditorCommand } from "../mediaEditorSettings";
import { closeSettings, openSettings } from "../ui";
import { Settings } from "./Settings";

const editor = MEDIA_EDITORS.find((entry) => entry.detectable)!;

afterEach(() => { closeSettings(); vi.restoreAllMocks(); document.body.innerHTML = ""; localStorage.clear(); });

it("keeps a command edited while the Settings autodetect request is pending", async () => {
  setMediaEditorCommand(editor.settingKey, "");
  vi.spyOn(backend(), "setAppString").mockResolvedValue();
  let finish!: (command: string) => void;
  vi.spyOn(backend(), "detectMediaEditor").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  localStorage.setItem("tine.settings.advanced.files", "1");
  const host = document.createElement("div");
  document.body.append(host);
  const dispose = render(() => <Settings />, host);
  try {
    openSettings("files");
    const button = () => [...host.querySelectorAll<HTMLButtonElement>(".media-editor-row button")]
      .find((item) => item.textContent?.trim() === "Autodetect");
    await vi.waitFor(() => expect(button()).not.toBeUndefined());
    const input = button()!.closest(".media-editor-row")!.querySelector<HTMLInputElement>("input")!;
    button()!.click();
    input.value = "my-editor %f";
    input.dispatchEvent(new Event("change", { bubbles: true }));
    finish("autodetected %f");
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(mediaEditorCommand(editor.settingKey)).toBe("my-editor %f");
  } finally { dispose(); }
});
