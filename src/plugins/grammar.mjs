/** Pure manifest ID/path contract, shared by host and package checker.
 * The schema is checked against these definitions in grammar.test.ts.
 * Keep native capabilities closed (ADR 0052).
 */
export const pluginGrammar = {
  packageId: {
    type: "string",
    pattern: "^(?=.*\\.)[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])?$(?![\\s\\S])",
  },
  contributionId: {
    type: "string",
    pattern: "^[a-z0-9][a-z0-9._-]{0,63}$(?![\\s\\S])",
  },
  entry: {
    type: "string",
    maxLength: 160,
    pattern: "^[A-Za-z0-9][A-Za-z0-9._/-]*\\.wasm$(?![\\s\\S])",
    not: { pattern: "(?:^|/)\\.\\.(?:/|$)" },
  },
};

const patterns = Object.fromEntries(Object.entries(pluginGrammar).map(([kind, rule]) => [kind, new RegExp(rule.pattern)]));
const traversal = new RegExp(pluginGrammar.entry.not.pattern);

/** Returns the original string, or null for malformed untrusted input. No I/O. */
export function parsePluginGrammar(value, kind) {
  if (typeof value !== "string" || !patterns[kind].test(value)) return null;
  if (kind === "entry" && (value.length > pluginGrammar.entry.maxLength || traversal.test(value))) return null;
  return value;
}
