// Shared state + helpers for the mobile capture buttons (camera / voice memo).
// Recording is a single app-wide state (one mic), toggled from the focused
// editor's toolbar button; the block editor owns the actual start/stop + insert.
import { createSignal } from "solid-js";

/** True while a voice-memo recording is in progress (drives the mic button's
 *  stop icon + pulsing state in the toolbar). */
export const [isRecordingAudio, setRecordingAudio] = createSignal(false);

export const DESKTOP_RECORDING_MAX_BYTES = 32 * 1024 * 1024;
export const DESKTOP_RECORDING_MAX_MS = 30 * 60 * 1000;

type DesktopRecordingCallbacks = {
  complete: (bytes: Uint8Array, mime: string, limited: boolean) => void | Promise<void>;
  error: (message: string) => void;
};

type DesktopRecordingSession = {
  owner: symbol;
  callbacks: DesktopRecordingCallbacks;
  stream?: MediaStream;
  recorder?: MediaRecorder;
  chunks: Blob[];
  bytes: number;
  cancelled: boolean;
  limited: boolean;
  timer?: number;
};

let desktopSession: DesktopRecordingSession | null = null;

function stopTracks(session: DesktopRecordingSession): void {
  session.stream?.getTracks().forEach((track) => track.stop());
  session.stream = undefined;
}

function clearSession(session: DesktopRecordingSession): void {
  if (session.timer !== undefined) window.clearTimeout(session.timer);
  session.timer = undefined;
  stopTracks(session);
  if (desktopSession === session) desktopSession = null;
  setRecordingAudio(false);
}

async function finishDesktopRecording(session: DesktopRecordingSession): Promise<void> {
  const chunks = session.chunks;
  session.chunks = [];
  clearSession(session);
  if (session.cancelled) return;
  try {
    const blob = new Blob(chunks, { type: session.recorder?.mimeType ?? "" });
    if (blob.size > DESKTOP_RECORDING_MAX_BYTES) {
      throw new Error("recording exceeded the 32 MiB limit");
    }
    const bytes = new Uint8Array(await blob.arrayBuffer());
    if (bytes.byteLength > DESKTOP_RECORDING_MAX_BYTES) {
      throw new Error("recording exceeded the 32 MiB limit");
    }
    if (bytes.byteLength) {
      await session.callbacks.complete(bytes, blob.type, session.limited);
    }
  } catch (error) {
    session.callbacks.error(String(error));
  }
}

export function desktopVoiceRecordingActive(): boolean {
  return desktopSession !== null;
}

export async function startDesktopVoiceRecording(
  owner: symbol,
  callbacks: DesktopRecordingCallbacks
): Promise<"started" | "busy"> {
  if (desktopSession) return "busy";
  if (!navigator.mediaDevices?.getUserMedia || typeof MediaRecorder === "undefined") {
    throw new Error("Mic capture isn’t available here");
  }
  // Reserve ownership before awaiting permission so two editors cannot race two
  // getUserMedia prompts into concurrent physical recorders.
  const session: DesktopRecordingSession = {
    owner,
    callbacks,
    chunks: [],
    bytes: 0,
    cancelled: false,
    limited: false,
  };
  desktopSession = session;
  try {
    const stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    if (desktopSession !== session || session.cancelled) {
      stream.getTracks().forEach((track) => track.stop());
      return "busy";
    }
    session.stream = stream;
    const recorder = new MediaRecorder(stream);
    session.recorder = recorder;
    recorder.ondataavailable = (event) => {
      if (session.cancelled || !event.data?.size) return;
      if (session.bytes + event.data.size > DESKTOP_RECORDING_MAX_BYTES) {
        session.limited = true;
        if (recorder.state !== "inactive") recorder.stop();
        return;
      }
      session.bytes += event.data.size;
      session.chunks.push(event.data);
      if (session.bytes >= DESKTOP_RECORDING_MAX_BYTES && recorder.state !== "inactive") {
        session.limited = true;
        recorder.stop();
      }
    };
    recorder.onerror = () => {
      callbacks.error("MediaRecorder failed");
      cancelDesktopVoiceRecording(owner);
    };
    recorder.onstop = () => void finishDesktopRecording(session);
    recorder.start(1000);
    session.timer = window.setTimeout(() => {
      session.limited = true;
      if (recorder.state !== "inactive") recorder.stop();
    }, DESKTOP_RECORDING_MAX_MS);
    setRecordingAudio(true);
    return "started";
  } catch (error) {
    session.cancelled = true;
    clearSession(session);
    throw error;
  }
}

export function stopDesktopVoiceRecording(): boolean {
  const session = desktopSession;
  const recorder = session?.recorder;
  if (!session || !recorder) return false;
  if (recorder.state !== "inactive") recorder.stop();
  return true;
}

export function cancelDesktopVoiceRecording(owner?: symbol): boolean {
  const session = desktopSession;
  if (!session || (owner !== undefined && session.owner !== owner)) return false;
  session.cancelled = true;
  if (session.timer !== undefined) window.clearTimeout(session.timer);
  const recorder = session.recorder;
  if (recorder) {
    recorder.ondataavailable = null;
    recorder.onerror = null;
    recorder.onstop = null;
    if (recorder.state !== "inactive") {
      try {
        recorder.stop();
      } catch {
        // Tracks are authoritative cleanup even if the backend rejects stop().
      }
    }
  }
  session.chunks = [];
  session.bytes = 0;
  clearSession(session);
  return true;
}
