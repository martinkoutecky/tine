/** Metadata for the existing asset lease, computed once from already-read
 * bytes. APNG's animation control chunk is refused before any browser decode. */
export function isStaticIconImage(bytes: Uint8Array): boolean {
  if (bytes[0] === 0xff && bytes[1] === 0xd8 && bytes[2] === 0xff) return true;
  const signature = [137, 80, 78, 71, 13, 10, 26, 10];
  if (!signature.every((v, i) => bytes[i] === v)) return false;
  const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
  let ihdr = false;
  for (let offset = 8; offset + 12 <= bytes.length;) {
    const length = view.getUint32(offset);
    if (length > bytes.length - offset - 12) return false;
    const type = String.fromCharCode(...bytes.subarray(offset + 4, offset + 8));
    if (type === "acTL") return false;
    if (type === "IHDR") ihdr = true;
    if (type === "IEND") return ihdr;
    offset += length + 12;
  }
  return false;
}
