import { afterEach, describe, expect, it, vi } from "vitest";
import { backend } from "./backend";
import { MEDIA_EDITORS } from "./mediaEditors";
import { initMediaEditorSettings, mediaEditorCommand, resolveMediaEditorCommand, setMediaEditorCommand } from "./mediaEditorSettings";

const editor = MEDIA_EDITORS.find((entry) => entry.detectable)!;

afterEach(() => vi.restoreAllMocks());

describe("media editor settings ownership", () => {
  it("does not replace a manual command with a late autodetect result", async () => {
    let finish!: (command: string) => void;
    vi.spyOn(backend(), "detectMediaEditor").mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    vi.spyOn(backend(), "setAppString").mockResolvedValue();
    setMediaEditorCommand(editor.settingKey, "");
    const pending = resolveMediaEditorCommand(editor);
    setMediaEditorCommand(editor.settingKey, "my-editor %f");
    finish("autodetected %f");
    expect(await pending).toBe("my-editor %f");
    expect(mediaEditorCommand(editor.settingKey)).toBe("my-editor %f");
  });

  it("does not replace a manual command with a late startup read", async () => {
    let finish!: (command: string) => void;
    vi.spyOn(backend(), "getAppString").mockImplementation((key) => key === editor.settingKey
      ? new Promise((resolve) => { finish = resolve; })
      : Promise.resolve(""));
    vi.spyOn(backend(), "setAppString").mockResolvedValue();
    const pending = initMediaEditorSettings();
    setMediaEditorCommand(editor.settingKey, "my-editor %f");
    finish("old-command %f");
    await pending;
    expect(mediaEditorCommand(editor.settingKey)).toBe("my-editor %f");
  });
});
