import { page_identity_key } from "./render/wasm/lsdoc_wasm.js";

/** Native refs::page_key via synchronous WASM: Rust Unicode trim, contextual
 * lowercase, one boundary slash per side, then NFC. O(name bytes), no IPC.
 * The parser must be initialized before calling; display names stay intact. */
export const pageIdentityKey = page_identity_key;
