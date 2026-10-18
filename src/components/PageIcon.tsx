import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import { acquireAssetBlob, assetVersion } from "../assetCache";
import { pageIconOwnerPath } from "../pageIconBatch";
import { pageIconAsset } from "../pageIconImage";
import { EmojiText } from "../render/emoji";

/** All icon surfaces share the existing bounded asset transport and refresh
 * signal. A refused/missing/undecodable image keeps today's text fallback. */
export function PageIcon(props: { value: string; name: string }) {
  const rel = createMemo(() => {
    if (!props.value.startsWith("!") && !props.value.startsWith("[[")) return null;
    const path = pageIconOwnerPath(props.name, props.value);
    return pageIconAsset(props.value, path);
  });
  const [src, setSrc] = createSignal("");
  createEffect(() => {
    const path = rel();
    setSrc("");
    if (!path) return;
    assetVersion(path);
    let active = true;
    let release: (() => void) | undefined;
    void acquireAssetBlob(path).then(lease => {
      if (!active) { lease.release(); return; }
      release = lease.release;
      if (lease.staticIcon) setSrc(lease.url);
    });
    onCleanup(() => { active = false; release?.(); });
  });
  return <Show when={src()} fallback={<EmojiText text={props.value} />}>
    <img data-page-image-icon src={src()} alt="" style={{ width: "1em", height: "1em", "object-fit": "contain", "vertical-align": "-0.12em" }} onError={() => setSrc("")} />
  </Show>;
}
