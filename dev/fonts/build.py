#!/usr/bin/env python3
"""Build Triton's font assets into crates/triton-design/fonts/.

Run with `mise run fonts` (inside the dev/fonts container, which has fonttools).

Why static instances: cosmic-text (iced's text engine) renders a variable font's
`wght` axis, but fontdb registers a variable font as one face at its default weight,
and cosmic-text only picks a family's face when the weight matches exactly. A request
for 600 against a variable font falls back to another family. So every family ships
as static files at the weights the design uses (WEIGHTS), each with the right OS/2
weight class.

- Text: Jost, Google Sans Flex and JetBrains Mono (OFL 1.1, no Reserved Font Names,
  so instances keep their family names) are instanced at WEIGHTS, other axes at their
  defaults.
- Icons: Material Symbols Rounded (Apache-2.0) is cut down to the names in icons.txt,
  pinned at wght 500, GRAD 0, opsz 24, and written at FILL 0 and FILL 1 as "Triton
  Symbols" and "Triton Symbols Filled" (cosmic-text cannot set FILL; the new names
  keep it apart from any system copy).
- symbols.codepoints lists name and codepoint per icon; build.rs makes the `Icon` enum.
"""

import glob
import os
import shutil
import sys
import urllib.request

from fontTools import subset
from fontTools.ttLib import TTFont
from fontTools.varLib import instancer

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
OUT = os.path.join(ROOT, "crates", "triton-design", "fonts")
# Downloads are cached; the mise task mounts target/fonts-src here.
CACHE = os.environ.get("FONTS_CACHE", os.path.join(ROOT, "target", "fonts-src"))

FONTS = "https://raw.githubusercontent.com/google/fonts/main/ofl"
SYMBOLS = "https://raw.githubusercontent.com/google/material-design-icons/master"
SOURCES = {
    "Jost[wght].ttf": f"{FONTS}/jost/Jost%5Bwght%5D.ttf",
    "Jost-OFL.txt": f"{FONTS}/jost/OFL.txt",
    "GoogleSansFlex[ROND,opsz,slnt,wdth,wght].ttf": f"{FONTS}/googlesansflex/GoogleSansFlex%5BROND,opsz,slnt,wdth,wght%5D.ttf",
    "GoogleSansFlex-OFL.txt": f"{FONTS}/googlesansflex/OFL.txt",
    "JetBrainsMono[wght].ttf": f"{FONTS}/jetbrainsmono/JetBrainsMono%5Bwght%5D.ttf",
    "JetBrainsMono-OFL.txt": f"{FONTS}/jetbrainsmono/OFL.txt",
    "MaterialSymbolsRounded.ttf": f"{SYMBOLS}/variablefont/MaterialSymbolsRounded%5BFILL,GRAD,opsz,wght%5D.ttf",
    "MaterialSymbolsRounded.codepoints": f"{SYMBOLS}/variablefont/MaterialSymbolsRounded%5BFILL,GRAD,opsz,wght%5D.codepoints",
    "MaterialSymbols-LICENSE.txt": f"{SYMBOLS}/LICENSE",
}

# (cached source, output stem). Instanced at every weight in WEIGHTS.
TEXT = [
    ("Jost[wght].ttf", "Jost"),
    ("GoogleSansFlex[ROND,opsz,slnt,wdth,wght].ttf", "GoogleSansFlex"),
    ("JetBrainsMono[wght].ttf", "JetBrainsMono"),
]
# The weights the type scale and components.css use. Adding one here needs the
# matching entry in crates/triton-design/src/fonts.rs.
WEIGHTS = {400: "Regular", 500: "Medium", 600: "SemiBold"}
ICON_WEIGHT = 500
LICENSES = ["Jost-OFL.txt", "GoogleSansFlex-OFL.txt", "JetBrainsMono-OFL.txt", "MaterialSymbols-LICENSE.txt"]


def fetch():
    os.makedirs(CACHE, exist_ok=True)
    for name, url in SOURCES.items():
        path = os.path.join(CACHE, name)
        if not os.path.exists(path):
            print("fetch", name)
            urllib.request.urlretrieve(url, path)
    return lambda name: os.path.join(CACHE, name)


def pin_all(font, **overrides):
    """Every axis at its default, except the overrides: a static instance."""
    axes = {a.axisTag: a.defaultValue for a in font["fvar"].axes}
    axes.update(overrides)
    return instancer.instantiateVariableFont(font, axes)


def set_names(font, family, style, weight):
    font["OS/2"].usWeightClass = weight
    names = font["name"]
    for rec in list(names.names):
        if rec.nameID in (16, 17, 21, 22, 25):
            names.removeNames(nameID=rec.nameID)
    for rec in list(names.names):
        args = (rec.platformID, rec.platEncID, rec.langID)
        if rec.nameID == 1:
            names.setName(family if style == "Regular" else f"{family} {style}", 1, *args)
        elif rec.nameID == 2:
            names.setName("Regular", 2, *args)
        elif rec.nameID == 4:
            names.setName(f"{family} {style}", 4, *args)
        elif rec.nameID == 6:
            names.setName(f"{family.replace(' ', '')}-{style}", 6, *args)
    # Typographic family/subfamily: what fontdb uses to group the weights as one family.
    names.setName(family, 16, 3, 1, 0x409)
    names.setName(style, 17, 3, 1, 0x409)


def family_name(font):
    names = font["name"]
    return (names.getDebugName(16) or names.getDebugName(1)).strip()


def build_text(src, stem):
    for weight, style in WEIGHTS.items():
        font = pin_all(TTFont(src), wght=weight)
        family = family_name(font)
        set_names(font, family, style, weight)
        out = os.path.join(OUT, f"{stem}-{style}.ttf")
        font.save(out)
        print(f"wrote {os.path.relpath(out, ROOT)} ({os.path.getsize(out)} bytes)")


def icon_names():
    with open(os.path.join(OUT, "icons.txt")) as f:
        names = [line.strip() for line in f if line.strip() and not line.startswith("#")]
    if len(set(names)) != len(names):
        sys.exit("icons.txt has duplicates")
    return names


def build_symbols(src, codepoints, filled):
    font = TTFont(src)
    # Subset first (fontTools mishandles gvar when subsetting an instanced font).
    options = subset.Options()
    options.layout_features = []  # icons are addressed by codepoint, not ligature
    options.name_IDs = ["*"]
    options.notdef_outline = True
    options.drop_tables += ["DSIG"]
    sub = subset.Subsetter(options)
    sub.populate(unicodes=codepoints)
    sub.subset(font)
    font = pin_all(font, FILL=1 if filled else 0, GRAD=0, opsz=24, wght=ICON_WEIGHT)
    family = "Triton Symbols Filled" if filled else "Triton Symbols"
    set_names(font, family, "Medium", ICON_WEIGHT)
    out = os.path.join(OUT, "TritonSymbols-Filled.ttf" if filled else "TritonSymbols.ttf")
    font.save(out)
    print(f"wrote {os.path.relpath(out, ROOT)} ({os.path.getsize(out)} bytes)")


def main():
    src = fetch()
    for old in glob.glob(os.path.join(OUT, "*.ttf")):
        os.remove(old)
    os.makedirs(os.path.join(OUT, "licenses"), exist_ok=True)
    for lic in LICENSES:
        shutil.copyfile(src(lic), os.path.join(OUT, "licenses", lic))
    for cached, stem in TEXT:
        build_text(src(cached), stem)

    table = {}
    with open(src("MaterialSymbolsRounded.codepoints")) as f:
        for line in f:
            name, cp = line.split()
            table.setdefault(name, int(cp, 16))
    names = icon_names()
    missing = [n for n in names if n not in table]
    if missing:
        sys.exit(f"not in Material Symbols: {' '.join(missing)}")
    with open(os.path.join(OUT, "symbols.codepoints"), "w") as f:
        f.write("# Generated by dev/fonts/build.py from icons.txt. Do not edit.\n")
        for n in names:
            f.write(f"{n} {table[n]:04x}\n")

    cps = sorted({table[n] for n in names})
    for filled in (False, True):
        build_symbols(src("MaterialSymbolsRounded.ttf"), cps, filled)


if __name__ == "__main__":
    main()
