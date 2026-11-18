// Quick share needs the native app (DESIGN §3.9): the browser preview and the
// test mock refuse every action, the listing included, with the `disabled` kind;
// a refused listing is how the interface knows links are not offered.
import type { QuickShareError, QuickShareListing } from "./quickShare/commands";
import type { Backend } from "./backendInterface";

const off = async (): Promise<never> => { throw { kind: "disabled" } satisfies QuickShareError; };

export const mockQuickShareCommands = {
  quickShareList: off as () => Promise<QuickShareListing>,
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
