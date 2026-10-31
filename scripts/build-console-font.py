#!/usr/bin/env python3
"""Generate the fixed ASCII atlas from the pinned Spleen bitmap source."""
import argparse
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCE = ROOT / 'userspace/console/font/spleen-12x24.bdf'
OUTPUT = ROOT / 'userspace/console/font/ascii.bin'

def atlas(text):
    glyphs = {}
    encoding = None
    bitmap = False
    rows = []
    for line in text.splitlines():
        if line.startswith('STARTCHAR '):
            encoding, bitmap, rows = None, False, []
        elif line.startswith('ENCODING '):
            encoding = int(line.split()[1])
        elif line.startswith('BBX '):
            if list(map(int, line.split()[1:])) != [12, 24, 0, -5]:
                raise ValueError('unexpected glyph geometry')
        elif line == 'BITMAP':
            bitmap = True
        elif line == 'ENDCHAR':
            if encoding is not None and 32 <= encoding <= 126:
                if encoding in glyphs or len(rows) != 24:
                    raise ValueError('duplicate or incomplete ASCII glyph')
                glyphs[encoding] = b''.join(int(row, 16).to_bytes(2, 'big') for row in rows)
            bitmap = False
        elif bitmap:
            if len(line) != 4 or int(line, 16) & 15:
                raise ValueError('invalid bitmap row')
            rows.append(line)
    if set(glyphs) != set(range(32, 127)):
        raise ValueError('incomplete ASCII atlas')
    return b''.join(glyphs[i] for i in range(32, 127))

if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    data = atlas(SOURCE.read_text())
    if args.check:
        if OUTPUT.read_bytes() != data:
            raise SystemExit('console font atlas differs from pinned source')
    else:
        OUTPUT.write_bytes(data)
