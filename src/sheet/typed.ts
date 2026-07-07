const PLAIN_DECIMAL_RE = /^[+-]?\d+(?:\.\d+)?$/;

export function isPlainDecimalNumber(value: string): boolean {
  return PLAIN_DECIMAL_RE.test(value);
}
