import { afterEach, describe, expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { render } from "solid-js/web";
import { setLightbox } from "../ui";
import { Lightbox } from "./Toasts";

afterEach(() => {
  setLightbox(null);
  document.head.innerHTML = "";
  document.body.innerHTML = "";
});

describe("lightbox geometry under graph custom.css (GH #319)", () => {
  it("keeps the full-screen image auto-sized when a later generic img rule fixes thumbnail width", () => {
    const app = document.createElement("style");
    app.textContent = readFileSync("src/styles/app.css", "utf8");
    document.head.appendChild(app);
    const custom = document.createElement("style");
    custom.textContent = "img { width: 320px; height: 120px; }";
    document.head.appendChild(custom);

    setLightbox("data:image/png;base64,AA==");
    const root = document.createElement("div");
    document.body.appendChild(root);
    const dispose = render(() => <Lightbox />, root);
    try {
      const image = root.querySelector(".lightbox-img") as HTMLImageElement;
      const style = getComputedStyle(image);
      expect(style.width).toBe("auto");
      expect(style.height).toBe("auto");
      expect(style.objectFit).toBe("contain");
    } finally {
      dispose();
    }
  });
});
