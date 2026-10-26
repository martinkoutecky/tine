"""Write src-tauri/icons/icon.icns from docs/app-icon-macos.svg's geometry (GH #747).

The macOS icon is docs/app-icon.svg inside Apple's icon grid: viewBox -31..287, so the
256-unit rounded square is ~824 of 1024 px with a transparent margin. Drawn with Pillow
at 4x and downsampled (no SVG renderer needed); run from the repository root.
"""
from PIL import Image, ImageDraw

SIZE, SS, VIEW, ORIGIN = 1024, 4, 318, -31
scale = SIZE * SS / VIEW
px = lambda v: (v - ORIGIN) * scale
img = Image.new("RGBA", (SIZE * SS,) * 2, (0, 0, 0, 0))
d = ImageDraw.Draw(img)
d.rounded_rectangle([px(0), px(0), px(256), px(256)], radius=56 * scale, fill="#10b981")
half = 9 * scale  # stroke-width 18 with round caps: each stroke is a capsule
for (x1, y1), (x2, y2) in [((96, 72), (96, 128)), ((128, 72), (128, 128)), ((160, 72), (160, 128)),
                           ((96, 128), (160, 128)), ((128, 128), (128, 188))]:
    d.rounded_rectangle([px(x1) - half, px(y1) - half, px(x2) + half, px(y2) + half], radius=half, fill="white")
icon = img.resize((SIZE, SIZE), Image.LANCZOS)
icon.save("src-tauri/icons/icon.icns", sizes=[(s, s) for s in (16, 32, 64, 128, 256, 512, 1024)])
print("wrote src-tauri/icons/icon.icns")
