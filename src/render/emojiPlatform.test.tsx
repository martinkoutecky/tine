import { afterEach, describe, expect, it } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { installEditableEmojiPlatform } from "../editableEmoji";
import { EmojiText, emojiSplit } from "./emoji";
import { renderedTextCaret } from "./spans";

const sample = "😀😐😑🤛❤️💛🧡🔶🔷❌🟢";
const sequences = "plain # * 012 café 中文 ❤️ 👨‍👩‍👧 👩🏽‍💻 🇨🇿 1️⃣ 🏳️‍🌈";
let dispose: (() => void) | undefined;
afterEach(() => {
  dispose?.();
  delete document.documentElement.dataset.editableEmoji;
});

describe("platform emoji display", () => {
  it.each([
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64)",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)",
    "Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X)",
  ])("uses native text for the editor's platform face: %s", (ua) => {
    installEditableEmojiPlatform(ua);
    const host = document.createElement("div");
    const [text, setText] = createSignal(sample);
    dispose = render(() => <EmojiText text={text()} />, host);
    expect(host.querySelector("img")).toBeNull();
    expect(host.textContent).toBe(sample);
    setText(sequences);
    expect(host.textContent).toBe(sequences);
    const emojiRuns = [...host.querySelectorAll(".emoji-native")];
    expect(emojiRuns.map((node) => node.textContent)).toEqual(
      emojiSplit(sequences).filter((part) => part.t === "emoji").map((part) => part.v),
    );
    // Native runs remain ordinary DOM text for selection and click-to-edit.
    const family = emojiRuns.find((node) => node.textContent === "👨‍👩‍👧")!;
    const caret = renderedTextCaret(host, family.firstChild!, "👨‍👩‍👧".length);
    expect(caret).toEqual({ text: sequences, caret: sequences.indexOf("👨‍👩‍👧") + "👨‍👩‍👧".length });
  });

  it.each([
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15",
    "Mozilla/5.0 (Linux; Android 15)",
    "unknown",
  ])("preserves the existing SVG display on %s", (ua) => {
    installEditableEmojiPlatform(ua);
    const host = document.createElement("div");
    dispose = render(() => <EmojiText text={sequences} />, host);
    expect(host.querySelector(".emoji-native")).toBeNull();
    expect(host.querySelectorAll("img.emoji").length).toBeGreaterThan(0);
    expect(renderedTextCaret(host, host, 0).text).toBe(sequences);
  });
});
