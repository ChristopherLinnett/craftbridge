"""Independent check (review V1-28): table tags of every face in the given font files.
Usage: python3 -I sfnt_tables.py <file-list>. Prints per-format counts and the faces that carry no
glyf/CFF /CFF2 outline table, with whether they carry hvgl."""
import struct, sys
def faces(path):
    with open(path, 'rb') as f:
        head = f.read(12)
        tag = head[:4]
        offs = [0]
        if tag == b'ttcf':
            n = struct.unpack('>I', head[8:12])[0]
            offs = list(struct.unpack('>%dI' % n, f.read(4 * n)))
        out = []
        for o in offs:
            f.seek(o)
            h = f.read(12)
            num = struct.unpack('>H', h[4:6])[0]
            d = f.read(16 * num)
            out.append({d[i*16:i*16+4].decode('latin1') for i in range(num)})
        return out
files = [l.rstrip('\n') for l in open(sys.argv[1]) if l.strip()]
total = 0; nodraw = []; hvgl = 0
for p in files:
    for i, t in enumerate(faces(p)):
        total += 1
        if 'hvgl' in t: hvgl += 1
        if not ({'glyf', 'CFF ', 'CFF2'} & t):
            nodraw.append((p, i, 'hvgl' in t, 'sbix' in t or 'CBDT' in t or 'COLR' in t))
print('files', len(files), 'faces', total, 'faces with hvgl', hvgl)
print('faces with none of glyf/CFF /CFF2:', len(nodraw))
by = {}
for p, i, h, c in nodraw: by.setdefault((p, h, c), 0); by[(p, h, c)] += 1
for (p, h, c), n in sorted(by.items()): print(' ', n, p.split('/RuntimeRoot')[-1], 'hvgl' if h else '-', 'colour/bitmap' if c else '-')
