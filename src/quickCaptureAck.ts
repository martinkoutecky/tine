export interface QuickCaptureRequest {
  id?: string;
  text: string;
  title?: string;
}

export interface QuickCaptureAck {
  id: string;
  ok: boolean;
}

export const QUICK_CAPTURE_ACK_TIMEOUT_MS = 5000;
export const QUICK_CAPTURE_ACK_MAX_RETRIES = 2;

let fallbackSeq = 0;

export function createQuickCaptureRequestId(): string {
  const randomUUID = globalThis.crypto?.randomUUID;
  if (typeof randomUUID === "function") return randomUUID.call(globalThis.crypto);
  fallbackSeq += 1;
  return `quick-capture-${Date.now()}-${fallbackSeq}`;
}

export function quickCaptureAckMatches(
  pendingId: string | null | undefined,
  ack: QuickCaptureAck | null | undefined
): ack is QuickCaptureAck {
  return !!pendingId && !!ack && ack.id === pendingId;
}

export function shouldRetryQuickCapture(attemptsStarted: number): boolean {
  return attemptsStarted <= QUICK_CAPTURE_ACK_MAX_RETRIES;
}
