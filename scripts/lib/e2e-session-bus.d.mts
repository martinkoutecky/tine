/** Launch one Linux journey on a private session bus, preserving its env/args. */
export function privateSessionLaunch(
  script: string,
  args?: string[],
  env?: NodeJS.ProcessEnv,
): { command: string; args: string[]; env: NodeJS.ProcessEnv };
/** Re-exec direct Linux runs once; reuse runner-owned sessions; no-op elsewhere. */
export function ensurePrivateSessionBus(): void;
