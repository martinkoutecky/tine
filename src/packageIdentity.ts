/** Plugin/theme manifest identity syntax, shared by local manifests and the signed
 * registry. Pure, O(value length), no I/O; false lets each caller retain its own
 * error wording. Storage path admission is a separate native policy: it allows
 * trailing hyphens in IDs but rejects dot-ended prereleases.
 */
const ID = /^[a-z0-9](?:[a-z0-9.-]{1,62}[a-z0-9])?$/;
const VERSION = /^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)(?:-[0-9A-Za-z.-]+)?$/;
export const isPackageId = (value: string): boolean => ID.test(value) && value.includes(".");
export const isPackageVersion = (value: string): boolean => VERSION.test(value);
