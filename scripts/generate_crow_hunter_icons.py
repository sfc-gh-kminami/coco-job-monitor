#!/usr/bin/env python3
"""
Generate 64x36px Retina pixel-art animated GIFs for CoCo Job Monitor menu bar.
Motif: Gothic Crow Hunter (tricorne hat, curved beak mask, tattered crow-feather cape).
100% binary-alpha crisp pixel art (no semi-transparent fringes in GIF89a!).
Outputs to ~/.snowflake/cortex/job-monitor/icons/:
  - idle.gif    (monochrome template silhouette, 3 frames)
  - run.gif     (monochrome template silhouette, 6 frames)
  - wait.gif    (full-color + crisp 1px dark rim + radiant amber bell/lantern, 4 frames)
  - trouble.gif (full-color + crisp 1px dark rim + pulsing crimson warning, 4 frames)
  - done.gif    (full-color + crisp 1px dark rim + emerald-cyan messenger lamp, 4 frames)
"""

import math
import os
from PIL import Image, ImageDraw

W, H = 64, 36
OUT_DIR = os.path.expanduser("~/.snowflake/cortex/job-monitor/icons")
os.makedirs(OUT_DIR, exist_ok=True)

TRANSPARENT = (0, 0, 0, 0)
BLACK = (0, 0, 0, 255)


def px(img: Image.Image, x: int, y: int, color):
    if 0 <= x < W and 0 <= y < H:
        img.putpixel((x, y), color)


def line_px(img: Image.Image, x0: int, y0: int, x1: int, y1: int, color, thickness: int = 1):
    steps = max(abs(x1 - x0), abs(y1 - y0), 1)
    for i in range(steps + 1):
        t = i / steps
        cx = round(x0 + (x1 - x0) * t)
        cy = round(y0 + (y1 - y0) * t)
        for dy in range(-(thickness // 2), thickness - (thickness // 2)):
            for dx in range(-(thickness // 2), thickness - (thickness // 2)):
                px(img, cx + dx, cy + dy, color)


def poly_px(img: Image.Image, pts, color):
    ImageDraw.Draw(img).polygon(pts, fill=color)


def draw_tricorne_and_beak(img: Image.Image, hx: int, hy: int, lean: int, fg, cutout=TRANSPARENT):
    crown_tip_x = hx - 1 - lean
    crown_tip_y = hy - 9
    poly_px(img, [
        (crown_tip_x, crown_tip_y),
        (hx - 6, hy - 3),
        (hx + 5, hy - 3),
    ], fg)

    poly_px(img, [
        (hx - 10, hy - 2),
        (hx - 4, hy - 4),
        (hx + 6, hy - 4),
        (hx + 9, hy - 6),
        (hx + 7, hy - 2),
        (hx - 7, hy - 1),
    ], fg)

    poly_px(img, [
        (hx - 4, hy - 2),
        (hx + 3, hy - 2),
        (hx + 3, hy + 3),
        (hx - 3, hy + 3),
    ], fg)

    # Long curved crow beak jutting sharply left-downward
    poly_px(img, [
        (hx - 3, hy - 1),
        (hx - 10, hy + 2),
        (hx - 11, hy + 4),
        (hx - 6, hy + 3),
        (hx - 3, hy + 3),
    ], fg)

    # Hat band crease + sharp eye slit
    line_px(img, hx - 4, hy - 3, hx + 4, hy - 3, cutout, 1)
    px(img, hx - 4, hy, cutout)
    px(img, hx - 3, hy, cutout)


def draw_feather_cape_running(img: Image.Image, shoulder_x: int, shoulder_y: int, phase: float, fg, cutout=TRANSPARENT):
    feather_lengths = [30, 34, 28, 23, 18]
    base_angles = [-0.08, 0.03, 0.16, 0.30, 0.45]

    for idx, (length, ang) in enumerate(zip(feather_lengths, base_angles)):
        wave = math.sin(phase * 2 * math.pi - idx * 0.9) * (2.4 + idx * 0.35)
        tip_x = round(shoulder_x + math.cos(ang) * length)
        tip_y = round(shoulder_y + math.sin(ang) * length + wave)
        mid_x = round(shoulder_x + length * 0.52)
        mid_y = round(shoulder_y + math.sin(ang) * length * 0.45 + wave * 0.55)

        poly_px(img, [
            (shoulder_x - 1, shoulder_y + idx * 2),
            (mid_x, mid_y - 2),
            (tip_x, tip_y),
            (mid_x - 1, mid_y + 1),
            (shoulder_x, shoulder_y + idx * 2 + 3),
        ], fg)

    for idx in range(4):
        wave = math.sin(phase * 2 * math.pi - idx * 0.9) * 2.0
        sx = shoulder_x + 10 + idx * 2
        sy = shoulder_y + 2 + idx * 3 + round(wave * 0.4)
        line_px(img, sx, sy, sx + 11, sy + 1 + (idx % 2), cutout, 1)

    # Detached crow feathers tumbling and drifting backward (x = 48..62)
    feathers = [
        (49 + int((phase * 13) % 13), 8 + int(math.sin(phase * 6.28) * 2)),
        (54 + int(((phase + 0.45) * 9) % 9), 16 + int(math.cos(phase * 6.28) * 2)),
        (46 + int(((phase + 0.72) * 15) % 15), 24 + int(math.sin((phase + 0.3) * 6.28) * 2)),
    ]
    for fx, fy in feathers:
        if fx < W - 2:
            px(img, fx, fy, fg)
            px(img, fx + 1, fy, fg)
            px(img, fx + 2, fy - 1, fg)


def draw_run_frame(frame_idx: int, total_frames: int = 6) -> Image.Image:
    img = Image.new("RGBA", (W, H), TRANSPARENT)
    phase = frame_idx / total_frames
    bob = round(math.sin(phase * 2 * math.pi * 2) * 1.4)

    hx, hy = 18, 12 + bob
    # Forward-leaning hunter torso
    poly_px(img, [
        (hx - 2, hy + 3),
        (hx + 4, hy + 2),
        (hx + 7, hy + 11),
        (hx, hy + 11),
    ], BLACK)

    # Dynamic wide 6-frame Castlevania/Bloodborne dash stride facing left!
    # Keyframes of (knee_dx, knee_dy, foot_dx, foot_dy) relative to hip (hx+3, hy+11)
    stride_poses = [
        (-5, 4, -9, 10),   # 0: Far front-left foot strike
        (-3, 5, -4, 11),   # 1: Weight plant
        (1, 5, 3, 10),     # 2: Push-off backward
        (5, 4, 10, 7),     # 3: High rear kick behind cloak!
        (2, 3, 5, 5),      # 4: Recovery tuck
        (-3, 3, -6, 6),    # 5: High forward knee drive
    ]
    hip_x, hip_y = hx + 3, hy + 11
    for leg_offset in (0, 3):
        kx, ky, fx, fy = stride_poses[(frame_idx + leg_offset) % 6]
        knee = (hip_x + kx, hip_y + ky)
        foot = (hip_x + fx, min(34, hip_y + fy))
        line_px(img, hip_x, hip_y, knee[0], knee[1], BLACK, 2)
        line_px(img, knee[0], knee[1], foot[0], foot[1], BLACK, 2)
        # Pointed hunter boot toe facing left
        line_px(img, foot[0] - 2, foot[1], foot[0] + 1, foot[1], BLACK, 1)

    # Curved Hunter's Blade swept forward-left
    hand_x, hand_y = hx - 4, hy + 7 + (frame_idx % 2)
    line_px(img, hx, hy + 5, hand_x, hand_y, BLACK, 2)
    poly_px(img, [
        (hand_x, hand_y),
        (hand_x - 8, hand_y + 4),
        (hand_x - 12, hand_y + 3),
        (hand_x - 7, hand_y + 1),
    ], BLACK)

    draw_feather_cape_running(img, hx + 3, hy + 2, phase, BLACK, TRANSPARENT)
    draw_tricorne_and_beak(img, hx, hy, lean=2, fg=BLACK, cutout=TRANSPARENT)
    return img


def draw_standing_hunter_base(img: Image.Image, hx: int, hy: int, cape_sway: int, fg, cutout):
    poly_px(img, [
        (hx - 2, hy + 3),
        (hx + 5, hy + 3),
        (hx + 12 + cape_sway, hy + 20),
        (hx + 7 + cape_sway, hy + 21),
        (hx + 3, hy + 19),
        (hx - 3, hy + 19),
    ], fg)

    for i in range(4):
        fx = hx + 6 + i * 2 + cape_sway
        fy = hy + 9 + i * 3
        poly_px(img, [(fx - 2, fy), (fx + 4, fy + 2), (fx, fy + 3)], fg)
        px(img, fx, fy + 1, cutout)

    line_px(img, hx - 1, hy + 18, hx - 2, 34, fg, 2)
    line_px(img, hx + 3, hy + 18, hx + 3, 34, fg, 2)
    line_px(img, hx - 4, 34, hx - 1, 34, fg, 1)
    line_px(img, hx + 1, 34, hx + 4, 34, fg, 1)
    draw_tricorne_and_beak(img, hx, hy, lean=0, fg=fg, cutout=cutout)


def add_dark_rim_outline(img: Image.Image, rim_color=(11, 17, 32, 255)) -> Image.Image:
    out = Image.new("RGBA", (W, H), TRANSPARENT)
    src = img.load()
    dst = out.load()
    for y in range(H):
        for x in range(W):
            if src[x, y][3] == 0:
                for dy, dx in ((-1, 0), (1, 0), (0, -1), (0, 1)):
                    nx, ny = x + dx, y + dy
                    if 0 <= nx < W and 0 <= ny < H and src[nx, ny][3] == 255:
                        dst[x, y] = rim_color
                        break
    for y in range(H):
        for x in range(W):
            if src[x, y][3] == 255:
                dst[x, y] = src[x, y]
    return out


def draw_pixel_ring_sparkles(img: Image.Image, cx: int, cy: int, radius: int, color, diag_color):
    """100% opaque pixel-art glow ring & chime sparkles (perfect for GIF89a binary alpha!)."""
    pts_cardinal = [(0, -radius), (0, radius), (-radius, 0), (radius, 0)]
    for dx, dy in pts_cardinal:
        px(img, cx + dx, cy + dy, color)
    d = max(2, round(radius * 0.72))
    for dx, dy in ((-d, -d), (d, -d), (-d, d), (d, d)):
        px(img, cx + dx, cy + dy, diag_color)


def draw_idle_frame(frame_idx: int) -> Image.Image:
    img = Image.new("RGBA", (W, H), TRANSPARENT)
    sway = [0, 1, 0][frame_idx % 3]
    hx, hy = 25, 12
    draw_standing_hunter_base(img, hx, hy, sway, BLACK, TRANSPARENT)
    line_px(img, hx - 3, hy + 10, hx - 8, hy + 18, BLACK, 1)
    # Small Messenger lamp stand at left
    line_px(img, 10, 21, 10, 34, BLACK, 1)
    line_px(img, 8, 34, 12, 34, BLACK, 1)
    poly_px(img, [(8, 20), (12, 20), (11, 16), (9, 16)], BLACK)
    if frame_idx == 1:
        px(img, 10, 14, BLACK)
    return img


def draw_wait_frame(frame_idx: int) -> Image.Image:
    """Hunter raising radiant AMBER Beckoning Bell / Lantern to summon the user!"""
    base = Image.new("RGBA", (W, H), TRANSPARENT)
    body_fg = (241, 245, 249, 255)
    inner_cut = (22, 32, 52, 255)
    hx, hy = 29, 12
    draw_standing_hunter_base(base, hx, hy, frame_idx % 2, body_fg, inner_cut)

    lx, ly = 12, 12 - (frame_idx % 2)
    line_px(base, hx - 2, hy + 5, lx + 2, ly + 1, body_fg, 2)
    img = add_dark_rim_outline(base)

    # Solid opaque Amber Bell + expanding pixel chime rings (zero semi-transparent GIF artifacts!)
    amber_outer = (217, 119, 6, 255)
    amber_mid = (245, 158, 11, 255)
    amber_core = (254, 240, 138, 255)

    d = ImageDraw.Draw(img)
    d.ellipse((lx - 4, ly - 4, lx + 4, ly + 4), fill=amber_outer)
    poly_px(img, [(lx - 2, ly - 2), (lx + 2, ly - 2), (lx + 3, ly + 2), (lx - 3, ly + 2)], amber_core)
    px(img, lx, ly + 3, amber_mid)

    ring_r = [6, 8, 9, 7][frame_idx % 4]
    draw_pixel_ring_sparkles(img, lx, ly, ring_r, amber_core, amber_mid)
    if frame_idx in (1, 2):
        line_px(img, lx, ly - ring_r - 2, lx, ly - ring_r, amber_core, 1)
    return img


def draw_trouble_frame(frame_idx: int) -> Image.Image:
    """Hunter down on one knee, sword planted, pulsing CRIMSON hazard beacon!"""
    base = Image.new("RGBA", (W, H), TRANSPARENT)
    body_fg = (226, 232, 240, 255)
    inner_cut = (30, 22, 32, 255)
    hx, hy = 27, 17 + (frame_idx % 2)

    poly_px(base, [
        (hx - 2, hy + 2),
        (hx + 6, hy + 2),
        (hx + 18, hy + 15),
        (hx + 4, hy + 16),
        (hx - 3, hy + 14),
    ], body_fg)
    line_px(base, hx - 2, hy + 12, hx - 6, 34, body_fg, 2)
    line_px(base, 13, 17, 13, 34, body_fg, 2)
    line_px(base, 10, 20, 16, 20, body_fg, 1)

    draw_tricorne_and_beak(base, hx, hy, lean=2, fg=body_fg, cutout=inner_cut)
    img = add_dark_rim_outline(base)

    # Crisp rimmed Crimson Warning Triangle + pixel shockwave sparks
    ax, ay = 13, 10 - (frame_idx % 2)
    poly_px(img, [(ax, ay - 6), (ax + 6, ay + 5), (ax - 6, ay + 5)], (127, 29, 29, 255))
    poly_px(img, [(ax, ay - 5), (ax + 5, ay + 4), (ax - 5, ay + 4)], (239, 68, 68, 255))
    line_px(img, ax, ay - 2, ax, ay + 1, (255, 255, 255, 255), 1)
    px(img, ax, ay + 3, (255, 255, 255, 255))

    if frame_idx in (1, 2):
        draw_pixel_ring_sparkles(img, ax, ay, 8, (252, 165, 165, 255), (239, 68, 68, 255))
    return img


def draw_done_frame(frame_idx: int) -> Image.Image:
    """Hunter standing calmly beside lit Emerald-Cyan Hunter's Dream lamp."""
    base = Image.new("RGBA", (W, H), TRANSPARENT)
    body_fg = (241, 245, 249, 255)
    inner_cut = (16, 36, 42, 255)
    hx, hy = 29, 12
    draw_standing_hunter_base(base, hx, hy, frame_idx % 2, body_fg, inner_cut)
    img = add_dark_rim_outline(base)

    lx, ly = 12, 16
    line_px(img, lx, ly + 4, lx, 34, (148, 163, 184, 255), 2)
    line_px(img, lx - 3, 34, lx + 3, 34, (148, 163, 184, 255), 1)
    d = ImageDraw.Draw(img)
    d.ellipse((lx - 4, ly - 4, lx + 4, ly + 4), fill=(5, 150, 105, 255))
    d.ellipse((lx - 2, ly - 2, lx + 2, ly + 2), fill=(167, 243, 208, 255))
    px(img, lx, ly, (255, 255, 255, 255))

    ring_r = [6, 7, 8, 6][frame_idx % 4]
    draw_pixel_ring_sparkles(img, lx, ly, ring_r, (110, 231, 183, 255), (56, 189, 248, 255))
    return img


def save_transparent_gif(frames, path: str, duration_ms: int):
    pal_frames = []
    for fr in frames:
        alpha = fr.getchannel("A")
        mask = Image.eval(alpha, lambda a: 255 if a < 128 else 0)
        p = fr.convert("RGB").convert("P", palette=Image.Palette.ADAPTIVE, colors=255)
        p.paste(255, mask)
        pal_frames.append(p)

    pal_frames[0].save(
        path,
        save_all=True,
        append_images=pal_frames[1:],
        duration=duration_ms,
        loop=0,
        transparency=255,
        disposal=2,
        optimize=False,
    )


def load_gif_frames_rgba(path: str):
    im = Image.open(path)
    out = []
    try:
        while True:
            out.append(im.convert("RGBA"))
            im.seek(im.tell() + 1)
    except EOFError:
        pass
    return out


def main():
    save_transparent_gif([draw_idle_frame(i) for i in range(3)], os.path.join(OUT_DIR, "idle.gif"), 600)
    save_transparent_gif([draw_run_frame(i, 6) for i in range(6)], os.path.join(OUT_DIR, "run.gif"), 95)
    save_transparent_gif([draw_wait_frame(i) for i in range(4)], os.path.join(OUT_DIR, "wait.gif"), 220)
    save_transparent_gif([draw_trouble_frame(i) for i in range(4)], os.path.join(OUT_DIR, "trouble.gif"), 280)
    save_transparent_gif([draw_done_frame(i) for i in range(4)], os.path.join(OUT_DIR, "done.gif"), 320)

    # Render inspection sheet directly from DECODED .gif files on disk!
    sheet = Image.new("RGBA", (1160, 560), (9, 13, 24, 255))
    d = ImageDraw.Draw(sheet)

    rows = [
        ("idle.gif (Template)", load_gif_frames_rgba(os.path.join(OUT_DIR, "idle.gif")), True),
        ("run.gif  (Template)", load_gif_frames_rgba(os.path.join(OUT_DIR, "run.gif")), True),
        ("wait.gif (Amber)", load_gif_frames_rgba(os.path.join(OUT_DIR, "wait.gif")), False),
        ("trouble.gif (Red)", load_gif_frames_rgba(os.path.join(OUT_DIR, "trouble.gif")), False),
        ("done.gif (Emerald)", load_gif_frames_rgba(os.path.join(OUT_DIR, "done.gif")), False),
    ]

    for r_idx, (label, frames, is_template) in enumerate(rows):
        y0 = 20 + r_idx * 104
        d.text((16, y0), label, fill=(148, 163, 184, 255))

        # Simulated Dark macOS Menu Bar
        d.rounded_rectangle((16, y0 + 22, 165, y0 + 70), radius=6, fill=(24, 28, 38, 255))
        f0 = frames[1 % len(frames)].copy()
        if is_template:
            pxs = f0.load()
            for yy in range(H):
                for xx in range(W):
                    if pxs[xx, yy][3] > 0:
                        pxs[xx, yy] = (248, 250, 252, 255)
        sheet.alpha_composite(f0, (26, y0 + 28))
        d.text((96, y0 + 40), "2", fill=(248, 250, 252, 255))

        # Simulated Light macOS Menu Bar
        d.rounded_rectangle((175, y0 + 22, 324, y0 + 70), radius=6, fill=(232, 236, 242, 255))
        sheet.alpha_composite(frames[1 % len(frames)], (185, y0 + 28))
        d.text((255, y0 + 40), "2", fill=(15, 23, 42, 255))

        # Full 2x unclipped frames from actual decoded GIFs
        for f_i, fr in enumerate(frames):
            disp = fr.copy()
            if is_template:
                pxs = disp.load()
                for yy in range(H):
                    for xx in range(W):
                        if pxs[xx, yy][3] > 0:
                            pxs[xx, yy] = (241, 245, 249, 255)
            big = disp.resize((W * 2, H * 2), Image.Resampling.NEAREST)
            bx = 340 + f_i * 134
            d.rounded_rectangle((bx - 2, y0 + 12, bx + 130, y0 + 88), radius=5, fill=(17, 25, 42, 255))
            sheet.alpha_composite(big, (bx, y0 + 14))

    sheet.save("/tmp/crow_hunter_preview.png")
    print("Verified decoded GIFs OK:", OUT_DIR)


if __name__ == "__main__":
    main()
