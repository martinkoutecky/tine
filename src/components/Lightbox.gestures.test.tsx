// GH #501: swipe between images / swipe down to close / tap rules in the real
// Lightbox component, driven by synthetic pointer events. Thresholds themselves
// are pinned in imageViewerGestures.test.ts; this proves the wiring.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { render } from "solid-js/web";
import { lightbox, lightboxIndex, openLightbox, setLightbox } from "../ui";
import { Lightbox } from "./Toasts";

let dispose: (() => void) | null = null;
let root: HTMLDivElement;
const A = "data:image/png;base64,AA==";
const B = "data:image/png;base64,AB==";
const C = "data:image/png;base64,AC==";

beforeEach(() => {
  vi.useFakeTimers();
  Object.defineProperty(HTMLElement.prototype, "clientWidth", { configurable: true, get: () => 400 });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", { configurable: true, get: () => 800 });
  root = document.createElement("div");
  document.body.appendChild(root);
});
afterEach(() => {
  vi.useRealTimers();
  dispose?.();
  dispose = null;
  setLightbox(null);
  document.body.innerHTML = "";
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientWidth;
  delete (HTMLElement.prototype as unknown as Record<string, unknown>).clientHeight;
});

function mount(gallery: string[], first = gallery[0]) {
  openLightbox(first, gallery);
  dispose = render(() => <Lightbox />, root);
}
const overlay = () => root.querySelector(".lightbox-overlay") as HTMLElement;
const img = () => root.querySelector(".lightbox-img") as HTMLElement;

function ptr(target: Element, type: string, x: number, y: number, t: number, pointerType = "touch", id = 1) {
  const e = new Event(type, { bubbles: true, cancelable: true });
  for (const [k, v] of Object.entries({ pointerType, pointerId: id, clientX: x, clientY: y, timeStamp: t })) {
    Object.defineProperty(e, k, { value: v });
  }
  target.dispatchEvent(e);
}
/** A drag that ends with a still pause, so the release speed is zero and only distance counts. */
function swipe(target: Element, dx: number, dy: number, ms: number) {
  ptr(target, "pointerdown", 200, 400, 0);
  for (let i = 1; i <= 10; i++) ptr(target, "pointermove", 200 + (dx * i) / 10, 400 + (dy * i) / 10, (ms * i) / 10);
  ptr(target, "pointermove", 200 + dx, 400 + dy, ms + 200);
  ptr(target, "pointerup", 200 + dx, 400 + dy, ms + 200);
}
/** A 40 ms touch tap; the fake clock moves with it so the double-tap wait is real. */
function tapAt(target: Element, x: number, y: number) {
  ptr(target, "pointerdown", x, y, 0);
  vi.advanceTimersByTime(40);
  ptr(target, "pointerup", x, y, 40);
}
function click(target: Element, t: number) {
  const e = new MouseEvent("click", { bubbles: true, cancelable: true });
  Object.defineProperty(e, "timeStamp", { value: t });
  target.dispatchEvent(e);
}

describe("image viewer gestures in the Lightbox (GH #501)", () => {
  it("a left swipe shows the next image; the badge follows", () => {
    mount([A, B, C]);
    expect(root.querySelector(".lightbox-count")?.textContent).toBe("1 / 3");
    swipe(overlay(), -250, 0, 800);
    expect(lightbox()).toBe(B);
    expect(lightboxIndex()).toBe(1);
    expect(root.querySelector(".lightbox-count")?.textContent).toBe("2 / 3");
    expect((img() as HTMLImageElement).src).toBe(B);
  });
  it("a right swipe shows the previous image, wrapping past the first with 3+ images", () => {
    mount([A, B, C]);
    swipe(overlay(), 250, 0, 800);
    expect(lightbox()).toBe(C);
  });
  it("a slow swipe of exactly half the width does not turn the page", () => {
    mount([A, B, C]);
    swipe(overlay(), -200, 0, 800);
    expect(lightbox()).toBe(A);
  });
  it("two images stop at the ends; one image shows no badge", () => {
    mount([A, B]);
    swipe(overlay(), 250, 0, 800); // previous of first: nothing
    expect(lightbox()).toBe(A);
    swipe(overlay(), -250, 0, 800);
    expect(lightbox()).toBe(B);
    swipe(overlay(), -250, 0, 800); // next of last: nothing
    expect(lightbox()).toBe(B);
    dispose!(); dispose = null; setLightbox(null);
    mount([A]);
    expect(root.querySelector(".lightbox-count")).toBeNull();
  });
  it("a vertical swipe closes in either direction past 0.4 of a third of the height; just under does not", () => {
    // 800 px viewport: 0.4 * 800/3 / 0.6 friction = 177.8 px of finger travel.
    mount([A, B]);
    swipe(overlay(), 0, 177, 800);
    expect(lightbox()).toBe(A);
    swipe(overlay(), 0, -177, 800);
    expect(lightbox()).toBe(A);
    swipe(overlay(), 0, 178, 800);
    expect(lightbox()).toBeNull();
    mount([A, B]);
    swipe(overlay(), 0, -178, 800);
    expect(lightbox()).toBeNull();
  });
  it("the click that follows a swipe does not close the viewer", () => {
    mount([A, B, C]);
    swipe(overlay(), -250, 0, 800);
    click(overlay(), 1010);
    expect(lightbox()).toBe(B);
  });
  it("a touch tap never closes: it toggles the controls, on the image and on the backdrop (PhotoSwipe tapAction)", () => {
    mount([A, B]);
    expect(overlay().classList.contains("lightbox-ui-hidden")).toBe(false);
    tapAt(img(), 200, 400);
    click(img(), 50);
    expect(overlay().classList.contains("lightbox-ui-hidden")).toBe(false); // still inside the double-tap wait
    vi.advanceTimersByTime(300);
    expect(overlay().classList.contains("lightbox-ui-hidden")).toBe(true);
    expect(lightbox()).toBe(A);
    tapAt(overlay(), 20, 20);
    click(overlay(), 400);
    vi.advanceTimersByTime(300);
    expect(overlay().classList.contains("lightbox-ui-hidden")).toBe(false);
    expect(lightbox()).toBe(A);
  });
  it("the close button closes (the tap path has no other way out besides drag, pinch and Back)", () => {
    mount([A]);
    const close = root.querySelector(".lightbox-close") as HTMLElement;
    ptr(close, "pointerdown", 380, 30, 0);
    ptr(close, "pointerup", 380, 30, 40);
    click(close, 50);
    expect(lightbox()).toBeNull();
  });
  it("a double-tap zooms the image (no close, no controls toggle) and the transform is applied", () => {
    mount([A]);
    tapAt(img(), 200, 400);
    click(img(), 50);
    vi.advanceTimersByTime(80);
    tapAt(img(), 200, 400);
    click(img(), 170);
    vi.advanceTimersByTime(1000);
    expect(lightbox()).toBe(A);
    expect(overlay().classList.contains("lightbox-ui-hidden")).toBe(false);
    expect(img().style.transform).toContain("scale(2.5)");
  });
  it("on a zoomed image a drag that starts at its edge and continues pans to the next image", () => {
    mount([A, B]);
    for (let i = 0; i < 2; i++) { tapAt(img(), 200, 400); vi.advanceTimersByTime(80); }
    vi.advanceTimersByTime(1000);
    expect(img().style.transform).toContain("scale(2.5)");
    // jsdom has no layout: the image is the 400x800 box here, so 2.5x pans 300 px each way.
    swipe(overlay(), -2000, 0, 800); // starts inside: pans to the edge, never turns
    expect(lightbox()).toBe(A);
    swipe(overlay(), -250, 0, 800); // starts at the edge: moves the strip, slow release past half the width
    expect(lightbox()).toBe(B);
    expect(img().style.transform).toBe("");
  });
  it("a mouse never drags or swipes, and a mouse click on the image still closes", () => {
    mount([A, B]);
    ptr(overlay(), "pointerdown", 200, 400, 0, "mouse");
    for (let i = 1; i <= 10; i++) ptr(overlay(), "pointermove", 200 - 12 * i, 400, i * 80, "mouse");
    ptr(overlay(), "pointerup", 80, 400, 800, "mouse");
    expect(lightbox()).toBe(A);
    ptr(img(), "pointerdown", 200, 400, 2000, "mouse");
    click(img(), 2010);
    expect(lightbox()).toBeNull();
  });
  it("opening a different image resets zoom", () => {
    mount([A, B]);
    for (let i = 0; i < 2; i++) { tapAt(img(), 200, 400); vi.advanceTimersByTime(80); }
    expect(img().style.transform).toContain("scale(2.5)");
    swipe(overlay(), -50, 0, 2000); // zoomed: pans instead of turning
    expect(lightbox()).toBe(A);
    setLightbox(B);
    expect(img().style.transform).toBe("");
  });
  it("a plain setLightbox(src) with no gallery still shows that one image", () => {
    setLightbox(C);
    dispose = render(() => <Lightbox />, root);
    expect((img() as HTMLImageElement).src).toBe(C);
    expect(root.querySelector(".lightbox-count")).toBeNull();
  });
});
