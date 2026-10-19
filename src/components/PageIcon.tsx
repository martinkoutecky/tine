import { createEffect, createMemo, createSignal, onCleanup, Show } from "solid-js";
import { acquireIconAssetBlob, iconAssetVersion } from "../assetCache";
import { pageIconOwnerPath } from "../pageIconBatch";
import { pageIconAsset, pageIconImageLink } from "../pageIconImage";
import { EmojiText } from "../render/emoji";

/** All icon surfaces share the existing bounded asset transport and refresh
 * signal. Refused/missing/undecodable images show no icon. */
export function PageIcon(props: { value: string; name: string }) {
  const image = createMemo(() => {
    if (!props.value.startsWith("!") && !props.value.startsWith("[[")) return { isImage: false, rel: null };
    const path = pageIconOwnerPath(props.name, props.value);
    const node = pageIconImageLink(props.value, path);
    return { isImage: !!node, rel: pageIconAsset(props.value, path, node) };
  });
  const [src, setSrc] = createSignal("");
  createEffect(() => {
    const path = image().rel;
    setSrc("");
    if (!path) return;
    iconAssetVersion();
    let active = true;
    let release: (() => void) | undefined;
    void acquireIconAssetBlob(path).then(lease => {
      if (!active) { lease.release(); return; }
      release = lease.release;
      if (lease.staticIcon) setSrc(lease.url);
      else lease.release();
    });
    onCleanup(() => { active = false; release?.(); });
  });
  return <Show when={src()} fallback={<Show when={!image().isImage}><EmojiText text={props.value} /></Show>}>
    <img data-page-image-icon src={src()} alt="" style={{ width: "1em", height: "1em", "object-fit": "contain", "vertical-align": "-0.12em" }} onError={() => setSrc("")} />
  </Show>;
}
