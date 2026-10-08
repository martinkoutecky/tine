export interface AppIdentity {
  identifier: string;
  productName: string;
  androidApplicationId: string;
  deployName: string;
}
export interface AppIdentitySwitch {
  ship: string;
  identities: Record<string, AppIdentity>;
}
export const ROOT: string;
export const SWITCH_FILE: string;
export function readSwitch(root?: string): AppIdentitySwitch;
export const SHIP: string;
export const IDENTITIES: Record<string, AppIdentity>;
export const IDENTITY: AppIdentity;
export const APP_ID: string;
export function deriveIdentityFiles(root: string, ship: string): Record<string, string>;
