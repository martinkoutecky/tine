import nordCss from "./nord.css?inline";
import solarizedCss from "./solarized.css?inline";
import gruvboxCss from "./gruvbox.css?inline";

export interface GalleryTheme {
  id: string;
  name: string;
  author: string;
  compat: "full" | "partial";
  modes: ("light" | "dark")[];
  css: string;
  thumbnail: string;
}

export const galleryThemes: GalleryTheme[] = [
  {
    id: "nord",
    name: "Nord",
    author: "Tine",
    compat: "full",
    modes: ["light", "dark"],
    css: nordCss,
    thumbnail: "/theme-thumbnails/nord.png",
  },
  {
    id: "solarized",
    name: "Solarized",
    author: "Tine",
    compat: "full",
    modes: ["light", "dark"],
    css: solarizedCss,
    thumbnail: "/theme-thumbnails/solarized.png",
  },
  {
    id: "gruvbox",
    name: "Gruvbox",
    author: "Tine",
    compat: "full",
    modes: ["light", "dark"],
    css: gruvboxCss,
    thumbnail: "/theme-thumbnails/gruvbox.png",
  },
];

export function galleryThemeById(id: string): GalleryTheme | undefined {
  return galleryThemes.find((theme) => theme.id === id);
}
