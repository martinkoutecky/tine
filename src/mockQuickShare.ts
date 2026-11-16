// Quick share needs the native app (DESIGN §3.9): the browser preview and the
// test mock report it off, and refuse every action with the `disabled` kind.
import { QUICK_SHARE_OFF, type QuickShareError, type QuickShareListing } from "./quickShare/commands";
import type { Backend } from "./backendInterface";

const off = async (): Promise<never> => { throw { kind: "disabled" } satisfies QuickShareError; };

export const mockQuickShareCommands = {
  async quickShareList(): Promise<QuickShareListing> { return QUICK_SHARE_OFF; },
  async quickShareTestConnection() { return "disabled" as const; },
  async quickShareDiscard(): Promise<void> {},
  async quickShareConnectCancel(): Promise<void> {},
  quickSharePrepare: off,
  quickShareCreate: off,
  quickShareCheck: off,
  quickShareFind: off,
  quickShareDelete: off,
  quickShareForget: off,
  quickShareCopyLink: off,
  quickShareConnectStart: off,
  quickShareConnectStatus: off,
  quickShareDisconnect: off,
} satisfies Partial<Backend>;
