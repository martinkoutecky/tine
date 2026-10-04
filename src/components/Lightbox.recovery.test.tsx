import { afterEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { renderInlines } from "../render/inline";
import { lightbox, setLightbox } from "../ui";
import { dismissTopTransient, topTransientLayer } from "../transientLayers";
import { Lightbox } from "./Toasts";

const inlineImage = (url: string) => renderInlines([{ k: "link", url: { type: "complex", protocol: "https", link: url.slice(8) }, full: `![image](${url})`, image: true, label: [{ k: "plain", text: "image" }] }]);

afterEach(() => {
  setLightbox(null);
  vi.useRealTimers();
  document.body.innerHTML = "";
});

describe("GH #566 image viewer recovery at the inline-image entry", () => {
  it.each(["backdrop", "close", "escape", "back"])("can exit through %s even after the preview image fails", (exit) => {
    const root = document.createElement("div");
    document.body.append(root);
    const action = vi.fn();
    const dispose = render(() => <>
      <button onClick={action}>Underlying action</button>
      {inlineImage("https://example.test/missing.png")}
      <Lightbox />
    </>, root);
    try {
      root.querySelector<HTMLImageElement>(".inline-image")!.click();
      expect(lightbox()).toBe("https://example.test/missing.png");
      root.querySelector(".lightbox-img")!.dispatchEvent(new Event("error"));
      if (exit === "escape" || exit === "back") dismissTopTransient(exit);
      else root.querySelector<HTMLElement>(exit === "close" ? ".lightbox-close" : ".lightbox-overlay")!.click();
      expect(lightbox()).toBeNull();
      expect(root.querySelector(".lightbox-overlay")).toBeNull();
      expect(topTransientLayer()?.id).not.toBe("image-lightbox");
      root.querySelector<HTMLButtonElement>("button")!.click();
      expect(action).toHaveBeenCalledOnce();
    } finally { dispose(); }
  });

  it("can close, reopen, and close after two rapid clicks on the inline image", () => {
    const root = document.createElement("div");
    document.body.append(root);
    const dispose = render(() => <>{inlineImage("https://example.test/image.png")}<Lightbox /></>, root);
    try {
      const inline = root.querySelector<HTMLImageElement>(".inline-image")!;
      inline.click(); inline.click();
      root.querySelector<HTMLElement>(".lightbox-close")!.click();
      inline.click();
      expect(root.querySelector(".lightbox-overlay")).not.toBeNull();
      dismissTopTransient("escape");
      expect(root.querySelector(".lightbox-overlay")).toBeNull();
    } finally { dispose(); }
  });
});
