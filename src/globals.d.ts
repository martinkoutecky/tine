// Build-time constants injected by Vite's `define` (see vite.config.ts).
declare const __BUILD_TIME__: string;
declare const __GIT_COMMIT__: string;
// False in F-Droid builds (TINE_COMMUNITY_REGISTRY=0): no network plugin/theme registry.
declare const __TINE_COMMUNITY_REGISTRY__: boolean;

interface Window {
  __tineApplyTheme?: (id: string) => void;
  __tineMockCustomCss?: string;
}

// KaTeX's mhchem extension ships no types; it's imported only for its global
// side-effect (registers \ce{…} on the katex instance).
declare module "katex/contrib/mhchem";
