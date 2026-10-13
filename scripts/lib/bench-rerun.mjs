// The disturbed round of one version, if any (bench-ab.mjs). A round is
// disturbed when a metric's spread across rounds exceeds that metric's
// reliability limit; the disturbed round is the one furthest (relatively)
// from the median of the round minima. Limits never change: a rerun replaces
// that one round with a fresh measurement, and the checker then judges the
// new distribution against the same budgets.

const median = (values) => {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};
const spreadPct = (values) => (Math.max(...values) / Math.min(...values) - 1) * 100;

/** @returns {{ round: number, metric: string, spreadPct: number } | null} */
export function disturbedRound(samples, metrics) {
  let worst = null;
  for (const [name, budget] of Object.entries(metrics)) {
    const mins = samples.map((sample) => sample.metrics[name].rawMin);
    const spread = spreadPct(mins);
    if (!(spread > budget.maxRoundSpreadPct)) continue;
    const mid = median(mins);
    let round = 0;
    for (let index = 1; index < mins.length; index++) {
      if (Math.abs(mins[index] / mid - 1) > Math.abs(mins[round] / mid - 1)) round = index;
    }
    const excess = spread / budget.maxRoundSpreadPct;
    if (!worst || excess > worst.excess) worst = { round, metric: name, spreadPct: spread, excess };
  }
  if (!worst) return null;
  const { excess: _excess, ...answer } = worst;
  return answer;
}
