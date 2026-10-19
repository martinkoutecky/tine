export type SurfaceEnv = "desktop" | "mobile";
export interface Surface { env: SurfaceEnv; budget: number; what: string; source: string }
export const SURFACES: Readonly<Record<string, Surface>>;
export function checkSurfaceBudgets(counts: Record<string, number>, env: SurfaceEnv, surfaces?: Readonly<Record<string, Surface>>): { slack: string[] };
