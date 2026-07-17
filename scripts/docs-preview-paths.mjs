import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

export function resolveRepositoryRoot(moduleUrl) {
  return dirname(dirname(fileURLToPath(moduleUrl)));
}

export function resolveViteExecutable(repoRoot, platform = process.platform) {
  return join(repoRoot, "node_modules", ".bin", platform === "win32" ? "vite.cmd" : "vite");
}

export function resolveLaunchPluginRoots(repoRoot, env = process.env) {
  return {
    bullet: env.TINE_BULLET_PLUGIN_ROOT
      ?? join(repoRoot, "community-plugins", "bullet-threading"),
    query: env.TINE_QUERY_PLUGIN_ROOT
      ?? join(repoRoot, "community-plugins", "query-filter"),
    heading: env.TINE_HEADING_PLUGIN_ROOT
      ?? join(repoRoot, "community-plugins", "heading-level-shortcuts"),
  };
}

export function resolveLaunchThemeRoots(repoRoot, env = process.env) {
  const checkoutParent = dirname(repoRoot);
  return {
    dev: env.TINE_DEV_THEME_ROOT ?? join(checkoutParent, "tine-theme-dev"),
    things: env.TINE_THINGS_THEME_ROOT ?? join(checkoutParent, "tine-theme-things"),
  };
}
