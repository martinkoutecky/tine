import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { toasts, dismissToast, lightbox, setLightbox, pushToast } from "../ui";
import { backend } from "../backend";

// Re-encode the shown image to PNG bytes (canvas), then hand them to the Rust
// clipboard writer. WebKitGTK's native "Copy Image" doesn't populate the OS
// clipboard, so we go through the backend instead.
async function copyLightboxImage(src: string): Promise<void> {
  const img = new Image();
  img.src = src;
  await img.decode();
  const canvas = document.createElement("canvas");
  canvas.width = img.naturalWidth;
  canvas.height = img.naturalHeight;
  const ctx = canvas.getContext("2d");
  if (!ctx) throw new Error("no 2d context");
  ctx.drawImage(img, 0, 0);
  const blob: Blob | null = await new Promise((r) => canvas.toBlob(r, "image/png"));
  if (!blob) throw new Error("encode failed");
  await backend().copyImageToClipboard(new Uint8Array(await blob.arrayBuffer()));
}

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
  // Esc closes the viewer (the right-click menu first, if it's open). Capture
  // phase + stopImmediatePropagation so it wins over any block-level Esc handler.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || !lightbox()) return;
      e.preventDefault();
      e.stopImmediatePropagation();
      if (menu()) setMenu(null);
      else setLightbox(null);
    };
    window.addEventListener("keydown", onKey, true);
    onCleanup(() => window.removeEventListener("keydown", onKey, true));
  });
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
  return (
    <Show when={lightbox()}>
      <div
        class="lightbox-overlay"
        onClick={() => {
          setMenu(null);
          setLightbox(null);
        }}
      >
        <img
          class="lightbox-img"
          src={lightbox()!}
          alt=""
          onContextMenu={(e) => {
            e.preventDefault();
            e.stopPropagation();
            setMenu({ x: e.clientX, y: e.clientY });
          }}
        />
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
