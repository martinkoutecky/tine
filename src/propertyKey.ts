// The ONE frontend property-key normaliser. It mirrors
// `tine_core::doc::property_key_norm` exactly (trim, ASCII-only lowercase, space
// and underscore to hyphen); `tests/fixtures/i12-property-key-norm-golden.json`
// is read by both the Rust and the TypeScript suite (I-12). Unicode case folding
// is deliberately absent: the engine stores and matches keys ASCII-folded, so a
// `toLowerCase()` here would name a column the engine never finds.
export function propertyKeyNorm(key: string): string {
  return key.trim().replace(/[A-Z]/g, (c) => c.toLowerCase()).replace(/[ _]/g, "-");
}
