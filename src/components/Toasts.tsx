import { For, Show, batch, createEffect, createMemo, createSignal, onCleanup, type JSX } from "solid-js";
import { toasts, dismissToast, pushToast } from "../toasts";
import { writeClipboardText } from "../clipboard";
import { lightbox, setLightbox, lightboxGallery, lightboxIndex, setLightboxIndex } from "../ui";
import { createImageViewerGestures, type Transform } from "../imageViewerGestures";
import { stepIndex } from "../imageGallery";
import { copyImageFromSrc as copyLightboxImage } from "../copyImage";
import { registerTransientLayer } from "../transientLayers";

// Bottom-right transient notifications.
export function Toasts(): JSX.Element {
  return (
    <div class="toast-stack">
      <For each={toasts()}>
        {(t) => (
          <div
            class={`toast toast-${t.kind}`}
            classList={{ "toast-sticky": t.sticky }}
            // Transient toasts dismiss on any click; sticky ones only via the ✕.
            onClick={() => !t.sticky && dismissToast(t.id)}
          >
            <span class="toast-msg">{t.message}</span>
            <Show when={(t.count ?? 1) > 1}>
              <span class="toast-count" aria-label={`shown ${t.count} times`}>×{t.count}</span>
            </Show>
            <Show when={t.action}>
              <button
                class="toast-action"
                onClick={(e) => {
                  e.stopPropagation();
                  t.action!.run();
                  dismissToast(t.id);
                }}
              >
                {t.action!.label}
              </button>
            </Show>
            <Show when={t.kind === "error"}>
              <button
                class="toast-action toast-copy"
                title="Copy the full message"
                onClick={(e) => {
                  e.stopPropagation();
                  // The error toast stays open; the copy result is a separate note.
                  void writeClipboardText(t.message).then(
                    () => pushToast("Error message copied", "success"),
                    () => pushToast("Couldn't copy the message; select its text instead.", "warn"),
                  );
                }}
              >
                Copy
              </button>
            </Show>
            <button
              class="toast-close"
              aria-label="Dismiss"
              onClick={(e) => {
                e.stopPropagation();
                dismissToast(t.id);
              }}
            >
              ×
            </button>
          </div>
        )}
      </For>
    </div>
  );
}

// Full-screen image viewer; click the backdrop to close. Right-click the image
// (or use the Copy button) to copy it to the OS clipboard.
export function Lightbox(): JSX.Element {
  const [menu, setMenu] = createSignal<{ x: number; y: number } | null>(null);
  let lastPointerType = "mouse";
  // Escape is owned by the application transient registry. Context-menu peeling
  // remains local to pointer interactions.
  const copy = async () => {
    setMenu(null);
    const src = lightbox();
    if (!src) return;
    try {
      await copyLightboxImage(src);
      pushToast("Image copied", "success");
    } catch {
      pushToast("Couldn't copy the image", "error");
    }
  };
  createEffect(() => {
    if (!lightbox()) return;
    const unregister = registerTransientLayer({
      id: "image-lightbox",
      root: () => document.querySelector<HTMLElement>(".lightbox-overlay"),
      dismiss: () => {
        if (menu()) { setMenu(null); return true; }
        setLightbox(null);
        return true;
      },
    });
    onCleanup(unregister);
  });
  // The page's images and the slot showing now (GH #501). A plain setLightbox(src)
  // that matches no slot shows that one image alone.
  const gallery = createMemo(() => {
    const list = lightboxGallery();
    const at = lightboxIndex();
    return list[at] !== undefined && list[at] === lightbox() ? list : lightbox() ? [lightbox()!] : [];
  });
  const slot = () => (gallery().length === 1 ? 0 : lightboxIndex());
  const [controls, setControls] = createSignal(true);
  const [view, setView] = createSignal<{ t: Transform; dx: number; dy: number }>({ t: { scale: 1, x: 0, y: 0 }, dx: 0, dy: 0 });
  let overlayEl: HTMLDivElement | undefined;
  let imgEl: HTMLImageElement | undefined;
  const gestures = createImageViewerGestures({
    box: () => ({ w: overlayEl?.clientWidth ?? 0, h: overlayEl?.clientHeight ?? 0 }),
    image: () => ({ w: imgEl?.clientWidth ?? 0, h: imgEl?.clientHeight ?? 0 }),
    apply: (t, drag) => setView({ t, dx: drag.x, dy: drag.y }),
    step: (delta) => {
      const next = stepIndex(slot(), gallery().length, delta);
      if (next === null) return false;
      // Read the target before writing: the gallery memo is derived from both signals.
      const target = gallery()[next];
      batch(() => {
        setLightboxIndex(next);
        setLightbox(target);
        setMenu(null);
      });
      return true;
    },
    close: () => { setMenu(null); setLightbox(null); },
    tap: () => setControls((v) => !v),
  });
  // A new image always opens at rest, with the controls showing (PhotoSwipe
  // starts with its UI visible; a tap toggles it, tapAction "toggle-controls").
  createEffect(() => { lightbox(); gestures.reset(); });
  createEffect(() => { if (!lightbox()) setControls(true); });
  // Touch / pen only: a mouse keeps its click-to-close and context menu, and
  // never drags the image. touch-action:none (CSS) hands us the whole gesture.
  const bind = (el: HTMLDivElement) => {
    overlayEl = el;
    const fromTouch = (e: PointerEvent) => e.pointerType === "touch" || e.pointerType === "pen";
    const on = (type: string, fn: (e: PointerEvent) => void) => el.addEventListener(type, fn as EventListener);
    on("pointerdown", (e) => {
      lastPointerType = e.pointerType;
      if (!fromTouch(e)) return;
      // Copy / close buttons and the menu keep their own taps.
      if ((e.target as HTMLElement | null)?.closest(".lightbox-copy, .lightbox-close, .lightbox-menu")) return;
      try { el.setPointerCapture?.(e.pointerId); } catch { /* synthetic pointer */ }
      gestures.down(e.pointerId, e.clientX, e.clientY, e.timeStamp);
    });
    on("pointermove", (e) => { if (fromTouch(e)) gestures.move(e.pointerId, e.clientX, e.clientY, e.timeStamp); });
    on("pointerup", (e) => { if (fromTouch(e)) gestures.up(e.pointerId, e.clientX, e.clientY, e.timeStamp); });
    on("pointercancel", (e) => { if (fromTouch(e)) gestures.cancel(e.pointerId); });
  };
  const imgStyle = () => {
    const { t, dx, dy } = view();
    const x = t.x + dx;
    const y = t.y + dy;
    return t.scale === 1 && x === 0 && y === 0 ? undefined : { transform: `translate(${x}px, ${y}px) scale(${t.scale})` };
  };
  return (
    <Show when={lightbox()}>
      <div
        class="lightbox-overlay"
        classList={{ "lightbox-ui-hidden": !controls() }}
        ref={bind}
        // PhotoSwipe's backdrop opacity during a vertical drag: 1 - |offset| / (viewport height / 3).
        style={{ "--lightbox-dim": String(Math.max(0, 1 - Math.abs(view().dy) / ((overlayEl?.clientHeight || 800) / 3))) }}
        onClick={(e) => {
          // A touch never closes by tapping (PhotoSwipe: a tap toggles the
          // controls; the viewer closes by drag, pinch, the close button or
          // Back). The click that follows a drag / pinch / double-tap is not a
          // close either. A mouse click on the image or backdrop closes.
          if (lastPointerType === "touch" || lastPointerType === "pen") return;
          if (gestures.swallowClick(e.timeStamp)) { e.stopPropagation(); return; }
          setMenu(null);
          setLightbox(null);
        }}
      >
        <img
          ref={imgEl}
          class="lightbox-img"
          src={lightbox()!}
          alt=""
          style={imgStyle()}
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setMenu({ x: e.clientX, y: e.clientY });
          }}
        />
        <Show when={gallery().length > 1}>
          <span class="lightbox-count" aria-hidden="true">{slot() + 1} / {gallery().length}</span>
        </Show>
        <button
          class="lightbox-close"
          title="Close"
          aria-label="Close image viewer"
          onClick={(e) => {
            e.stopPropagation();
            setMenu(null);
            setLightbox(null);
          }}
        >
          ×
        </button>
        <button
          class="lightbox-copy"
          title="Copy image to clipboard"
          onClick={(e) => {
            e.stopPropagation();
            void copy();
          }}
        >
          Copy
        </button>
        <Show when={menu()}>
          <div
            class="lightbox-menu"
            style={{ left: `${menu()!.x}px`, top: `${menu()!.y}px` }}
            onClick={(e) => e.stopPropagation()}
          >
            <button onClick={() => void copy()}>Copy image</button>
          </div>
        </Show>
      </div>
    </Show>
  );
}
