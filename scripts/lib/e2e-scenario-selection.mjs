/** Select the registry's workflow fan-out or an explicit journey. Local whole-suite
 * runs include by-name rows. Pure, O(rows); no build or application IO. */
export function selectScenarios(rows, only, workflow = false) {
  if (only) return rows.filter(([id]) => id === only);
  return workflow ? rows.filter((row) => row[3] !== BY_NAME) : rows;
}
export const BY_NAME = "by-name";

