/** Shared schema guards (I-12). O(keys) for knownKeys, O(text) only for
 * plain text; diagnostics and control policy belong to the caller's schema.
 * No cloning or additional validation pass. Error constructors run on failure. */
export function schemaGuards(
  ErrorType: new (message: string) => Error,
  policy: { object: (where: string) => string; string: (where: string, max: number) => string; plainText: boolean },
) {
  const record = (value: unknown, where: string): Record<string, unknown> => {
    if (!value || typeof value !== "object" || Array.isArray(value)) throw new ErrorType(policy.object(where));
    return value as Record<string, unknown>;
  };
  const knownKeys = (obj: Record<string, unknown>, where: string, allowed: readonly string[]): void => {
    const known = new Set(allowed);
    const unknown = Object.keys(obj).find((key) => !known.has(key));
    if (unknown) throw new ErrorType(`${where} contains unknown field ${unknown}`);
  };
  const text = (value: unknown, where: string, max = 500): string => {
    if (typeof value !== "string" || value.length === 0 || value.length > max
        || (policy.plainText && /[\u0000-\u001f]/.test(value))) throw new ErrorType(policy.string(where, max));
    return value;
  };
  return { record, knownKeys, text };
}
