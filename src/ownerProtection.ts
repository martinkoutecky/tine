export type OwnerProtectionIssue = {
  reason: "permissions" | "namespace" | "lock" | "handover";
  details: string;
  repairable: boolean;
  instructions: "permissions" | "flatpak" | "access" | "windows-access";
  command: string;
};
