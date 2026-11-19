export async function sampleThreadInk({ src, rows, dpr, zoom }) {
  const image = new Image(); image.src = src; await image.decode();
  const canvas = document.createElement("canvas"); canvas.width = image.width; canvas.height = image.height;
  const ctx = canvas.getContext("2d"); ctx.drawImage(image, 0, 0);
  const pixels = ctx.getImageData(0, 0, canvas.width, canvas.height).data;
  return rows.map(row => {
    const left = Math.floor(row.x * dpr) - 2;
    const right = Math.ceil((row.x + row.width) * dpr) + 2;
    const contrast = (x, y) => {
      const background = 4 * (y * canvas.width + left - 2);
      const k = 4 * (y * canvas.width + x);
      return Math.abs(pixels[background] - pixels[k]) + Math.abs(pixels[background + 1] - pixels[k + 1]) + Math.abs(pixels[background + 2] - pixels[k + 2]);
    };
    const rail = y => Array.from({ length: right - left + 1 }, (_, i) => contrast(left + i, y));
    const mass = y => rail(y).reduce((total, pixel) => total + pixel, 0);
    const first = Math.floor((Math.min(row.joinY, row.top) - zoom) * dpr);
    const last = Math.ceil((row.joinY + zoom) * dpr);
    const branchY = Math.floor(row.bulletY * dpr);
    const beforeBranch = Math.floor((row.bulletY - 5 * zoom) * dpr);
    let fork = null;
    if (row.radius === 0 && row.continuationVisible) {
      const spread = Math.ceil(row.width * dpr);
      const columns = Array.from({ length: right + 3 - left }, (_, i) => {
        let peak = 0;
        for (let y = branchY - spread; y <= branchY + spread; y++) peak = Math.max(peak, contrast(left + i, y));
        return peak;
      });
      const railPeak = Math.max(...columns.slice(0, Math.ceil((row.x + row.width) * dpr) - left + 1));
      fork = { peak: Math.max(...columns), arm: columns.at(-1), rail: Math.max(...rail(first - 2)),
        bridge: columns.slice(columns.indexOf(railPeak)) };
    }
    return { id: row.id, first, above: mass(first - 2), below: mass(last + 2),
      join: Array.from({ length: last - first + 1 }, (_, i) => mass(first + i)), fork,
      activeRail: row.focused && row.radius === 0 ? mass(beforeBranch) : null };
  });
}

export function hasPaleThreadFork(fork) {
  return Math.min(...fork.bridge) < .85 * Math.min(fork.bridge[0], fork.arm);
}

export function hasDarkThreadFork(fork) {
  return fork.peak > 1.15 * Math.max(fork.rail, fork.arm);
}

export function hasUnevenThreadSeam({ above, below, join }) {
  // A palette transition can have different ink on either side of a join.
  return !above || !below || Math.min(...join) < .85 * Math.min(above, below)
    || Math.max(...join) > 1.15 * Math.max(above, below);
}
