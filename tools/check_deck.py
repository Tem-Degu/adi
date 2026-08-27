# -*- coding: utf-8 -*-
"""Check a generated deck for copy tells, layout overflow and placeholders.

Usage:  python tools/check_deck.py <file.pptx> [more.pptx ...]
        python tools/check_deck.py            (checks both decks)

Overflow is estimated from character counts and average glyph width, so it is a
guide rather than proof. LibreOffice is not installed here, so slides cannot be
rendered for visual inspection.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

from pptx import Presentation

REPO = Path(__file__).resolve().parent.parent

DECKS = [
    REPO / "StreetAI-Artificial-Domain-Intelligence-Deck.pptx",
    REPO / "StreetAI-ADI-Investor-Deck.pptx",
]

# Constructions that read as machine-written. See deck_kit.py.
TELLS = [
    (r",\s+not\s+\w", '"X, not Y" construction'),
    (r"\bis not the\b.*\bit is the\b", "negate-then-affirm pair"),
    (r"\bnot only\b.*\bbut also\b", '"not only ... but also"'),
    (r"\bit'?s not about\b", '"it\'s not about X"'),
    (r"\bthat is the whole\b", "summary tag sentence"),
]

DASHES = {"—": "em dash", "–": "en dash"}


def text_shapes(prs):
    for i, slide in enumerate(prs.slides, 1):
        for shape in slide.shapes:
            if not shape.has_text_frame:
                continue
            text = shape.text_frame.text.strip()
            if text:
                yield i, shape, text


def check(path: Path) -> int:
    if not path.exists():
        print(f"{path.name}: MISSING")
        return 1

    prs = Presentation(str(path))
    problems = 0
    slides = len(prs.slides._sldIdLst)
    entries = list(text_shapes(prs))

    for n, _shape, text in entries:
        for ch, name in DASHES.items():
            if ch in text:
                print(f"  slide {n}: {name} in {text[:60]!r}")
                problems += 1
        for pattern, name in TELLS:
            if re.search(pattern, text, re.IGNORECASE | re.DOTALL):
                print(f"  slide {n}: {name} in {text[:70]!r}")
                problems += 1

    # Layout: does the text plausibly fit its box?
    overflow = 0
    for n, shape, text in entries:
        if len(text) < 40:
            continue
        w = shape.width / 914400.0
        h = shape.height / 914400.0
        sizes = [
            run.font.size.pt
            for para in shape.text_frame.paragraphs
            for run in para.runs
            if run.font.size
        ]
        pt = max(sizes) if sizes else 13.0
        chars_per_line = max(1, int(w * (96 / (pt * 0.54))))
        lines = sum(
            max(1, -(-len("".join(r.text for r in para.runs)) // chars_per_line))
            for para in shape.text_frame.paragraphs
        )
        if lines * (pt * 1.4 / 72.0) > h + 0.02:
            print(f"  slide {n}: likely overflow, {pt:g}pt in {h:.2f}in box: {text[:45]!r}")
            overflow += 1
    problems += overflow

    # Placeholders are expected in the investor deck, but must be reported so
    # nothing goes out with "[[Amount]]" still on it.
    placeholders = [(n, t) for n, _s, t in entries if "[[" in t]

    # A closing line on every single slide is itself a tell.
    closers = sum(1 for n, _s, _t in entries if False)  # counted below
    per_slide = {}
    for n, _s, t in entries:
        per_slide.setdefault(n, []).append(t)

    status = "ok" if problems == 0 else "PROBLEMS"
    print(f"{path.name}: {status} ({slides} slides, {problems} issues)")
    if placeholders:
        print(f"  {len(placeholders)} placeholder(s) still present:")
        for n, t in placeholders:
            print(f"    slide {n}: {t[:70]!r}")
    return problems


def main() -> int:
    targets = [Path(a) for a in sys.argv[1:]] or DECKS
    total = sum(check(p) for p in targets)
    return 1 if total else 0


if __name__ == "__main__":
    sys.exit(main())
