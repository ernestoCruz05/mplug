#!/usr/bin/env python3
import sys
import os
import urllib.request
from PIL import Image, ImageDraw, ImageOps

def main():
    if len(sys.argv) < 2:
        print("Usage: media-renderer.py <art_url_or_path> [dest_prefix]")
        sys.exit(1)

    art_source = sys.argv[1]
    dest_prefix = sys.argv[2] if len(sys.argv) > 2 else "/tmp/mplug_media"
    
    assets_dir = "/home/sof/.config/mplug/plugins/personal/assets/volume-controller"
    cat_frame_paths = [
        os.path.join(assets_dir, "cat_med_1.png"),
        os.path.join(assets_dir, "cat_med_2.png")
    ]
    
    local_art_path = None
    art_img = None
    
    if art_source:
        if art_source.startswith("http://") or art_source.startswith("https://"):
            try:
                temp_download_path = "/tmp/mplug_downloaded_art.png"
                urllib.request.urlretrieve(art_source, temp_download_path)
                local_art_path = temp_download_path
            except Exception as e:
                print(f"Failed to download album art: {e}", file=sys.stderr)
        elif art_source.startswith("file://"):
            local_art_path = art_source[7:]
        else:
            local_art_path = art_source
            
    if local_art_path and os.path.exists(local_art_path):
        try:
            art_img = Image.open(local_art_path).convert("RGBA")
        except Exception as e:
            print(f"Failed to open album art: {e}", file=sys.stderr)
            
    if art_img is None:
        art_img = Image.new("RGBA", (300, 300), (40, 40, 40, 255))
        draw_placeholder = ImageDraw.Draw(art_img)
        draw_placeholder.ellipse([(120, 120), (180, 180)], fill=(80, 80, 80, 255))
        draw_placeholder.rectangle([(160, 60), (170, 150)], fill=(80, 80, 80, 255))
        draw_placeholder.rectangle([(160, 60), (200, 90)], fill=(80, 80, 80, 255))

    vinyl_size = 64
    art_square = ImageOps.fit(art_img, (vinyl_size, vinyl_size), Image.Resampling.LANCZOS)
    
    mask = Image.new("L", (vinyl_size, vinyl_size), 0)
    draw_mask = ImageDraw.Draw(mask)
    draw_mask.ellipse((0, 0, vinyl_size, vinyl_size), fill=255)
    
    vinyl_base = Image.new("RGBA", (vinyl_size, vinyl_size), (0, 0, 0, 0))
    vinyl_base.paste(art_square, (0, 0), mask=mask)
    
    draw_vinyl_border = ImageDraw.Draw(vinyl_base)
    draw_vinyl_border.ellipse((0, 0, vinyl_size - 1, vinyl_size - 1), outline=(255, 255, 255, 255), width=1)
    
    cat_frames = []
    for path in cat_frame_paths:
        if os.path.exists(path):
            cat_frames.append(Image.open(path).convert("RGBA"))
        else:
            fallback = Image.new("RGBA", (80, 80), (0, 0, 0, 0))
            cat_frames.append(fallback)

    for i in range(12):
        canvas = Image.new("RGBA", (128, 128), (0, 0, 0, 0))
        
        cat_y = 0 if (i % 2 == 0) else 6
        cat_head = cat_frames[i % 2]
        canvas.paste(cat_head, (24, cat_y), mask=cat_head)
        rotated_vinyl = vinyl_base.rotate(-i * 30, Image.Resampling.BICUBIC)
        draw_rotated = ImageDraw.Draw(rotated_vinyl)
        draw_rotated.ellipse((29, 29, 35, 35), fill=(0, 0, 0, 255), outline=(255, 255, 255, 255), width=1)
        canvas.paste(rotated_vinyl, (32, 64), mask=rotated_vinyl)
        
        out_path = f"{dest_prefix}_{i}.png"
        canvas.save(out_path)

    if local_art_path == "/tmp/mplug_downloaded_art.png" and os.path.exists(local_art_path):
        os.remove(local_art_path)

if __name__ == "__main__":
    main()
