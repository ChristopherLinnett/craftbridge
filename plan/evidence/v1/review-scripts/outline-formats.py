"""Lists the faces under a simulator runtime root whose outlines are in a format other than the
ones EffectCraft can draw (TrueType glyf, CFF, CFF2 and VARC outlines, the formats of skrifa's
OutlineGlyphFormat; sbix/CBDT/EBDT bitmaps; COLR colour glyphs), by reading each face's table
directory, and counts the faces that carry Apple's hvgl outlines.
Usage: python3 -I outline-formats.py <RuntimeRoot>"""
import os
import struct
import sys

root = sys.argv[1]
folders = [root + "/System/Library/Fonts", root + "/System/Library/PrivateFrameworks/FontServices.framework"]
drawable = {"glyf", "CFF ", "CFF2", "VARC", "sbix", "CBDT", "EBDT", "COLR"}
hvgl_files, undrawable_files = set(), set()
total = hvgl_faces = undrawable_faces = both_faces = 0
for folder in folders:
    for dirpath, _, names in os.walk(folder):
        for name in names:
            if not name.lower().endswith((".ttf", ".ttc", ".otf", ".otc")):
                continue
            path = os.path.join(dirpath, name)
            with open(path, "rb") as f:
                head = f.read(12)
                offsets = [0]
                if head[:4] == b"ttcf":
                    n = struct.unpack(">I", head[8:12])[0]
                    offsets = struct.unpack(">%dI" % n, f.read(4 * n))
                for o in offsets:
                    f.seek(o)
                    count = struct.unpack(">H", f.read(12)[4:6])[0]
                    tags = {f.read(16)[:4].decode("latin1") for _ in range(count)}
                    total += 1
                    short = path.replace(root, "")
                    has_hvgl, draws = "hvgl" in tags, bool(tags & drawable)
                    hvgl_faces += has_hvgl
                    undrawable_faces += not draws
                    both_faces += has_hvgl and not draws
                    if has_hvgl:
                        hvgl_files.add(short)
                    if not draws:
                        undrawable_files.add(short)
print("faces scanned:", total)
print("files with hvgl faces:", sorted(hvgl_files))
print("files with faces lacking glyf/CFF/CFF2/VARC/sbix/CBDT/EBDT/COLR:", sorted(undrawable_files))
print("faces with hvgl:", hvgl_faces)
print("faces lacking every drawable table:", undrawable_faces)
print("faces with hvgl AND lacking every drawable table:", both_faces)
