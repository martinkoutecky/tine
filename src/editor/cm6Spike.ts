/** Branch-only build switch, with same-binary opt-out for comparison probes. */
export function cm6SpikeEnabled(): boolean {
  if (import.meta.env.MODE === "test" || typeof location === "undefined") return false;
  const choice = new URLSearchParams(location.search).get("cm6") ?? sessionStorage.getItem("tine-spike-cm6");
  return import.meta.env.VITE_CM6_SPIKE === "1" ? choice !== "0" : import.meta.env.DEV && choice === "1";
}
