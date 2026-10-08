// Frontend display identity derives from the same switch as native packaging.
// I-12: clients never spell their own channel name. See docs/app-identity.md.
import identitySwitch from "../src-tauri/app-identity.json";

export const APP_PRODUCT_NAME = identitySwitch.identities[identitySwitch.ship as keyof typeof identitySwitch.identities].productName;

/** The updater channel this build reads: the released identity is stable, the experiment is
 * Beta. src/appIdentity.guard.test.ts pins it against the updater endpoint and the version. */
export const APP_UPDATE_CHANNEL: "stable" | "beta" = identitySwitch.ship === "release" ? "stable" : "beta";
