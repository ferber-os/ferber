# Spleen bitmap source

This directory vendors Spleen 12×24 from upstream revision
`5116f8eac7f65da2cfa169626ec43b5d041665fe`:
https://github.com/fcambus/spleen

The original bitmap source and BSD 2-Clause license are retained. The ASCII atlas
contains glyphs 32–126, each with 24 big-endian two-byte rows. The high twelve bits
are pixels; the low four bits are padding.

Regenerate with `python3 scripts/build-console-font.py` from the repository root.
Use `--check` to verify that the checked-in atlas matches the source. Rendering
uses the atlas at its native size without runtime font parsing or vector state.
