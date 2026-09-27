#!/usr/bin/env python3
"""Fail if domain vocabulary has leaked into the domain-neutral crates.

The neutral crates must not depend structurally on any one domain. Doc comments
may mention a domain to explain an abstraction, so comments are stripped before
searching. Anything remaining is code, and code naming a domain is a leak.

Run before any commit touching a neutral crate. See sdk/README.md.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

NEUTRAL_CRATES = [
    "crates/adi-protocol",
    "crates/adi-safety",
    "crates/adi-core",
    "crates/adi-decision",
    "crates/adi-language",
    "crates/adi-wire",
    "crates/adi-session",
    "crates/adi-models",
]

# Vocabulary belonging to a specific domain rather than to the abstraction.
FORBIDDEN = [
    "shot", "espresso", "coffee", "barista", "grind", "grinder", "brew",
    "puck", "portafilter", "tamp", "bean", "roast", "crema", "extraction",
    "milk", "sugar", "syrup", "cup",
    # second-domain vocabulary, forbidden for the same reason
    "greenhouse", "irrigation", "substrate", "nutrient", "photoperiod",
]

PATTERN = re.compile(r"\b(" + "|".join(FORBIDDEN) + r")\w*", re.IGNORECASE)


def strip_comments(source: str) -> list[tuple[int, str]]:
    """Return (line_number, code_only) pairs with comments blanked out.

    Handles // line comments, /// and //! doc comments, and /* */ blocks.
    Leaves string literals intact, because a domain name in a string literal is
    a real leak.
    """
    out: list[tuple[int, str]] = []
    in_block = False
    for number, line in enumerate(source.splitlines(), start=1):
        code = line
        if in_block:
            end = code.find("*/")
            if end == -1:
                out.append((number, ""))
                continue
            code = code[end + 2 :]
            in_block = False
        while True:
            start = code.find("/*")
            if start == -1:
                break
            end = code.find("*/", start + 2)
            if end == -1:
                code = code[:start]
                in_block = True
                break
            code = code[:start] + " " + code[end + 2 :]
        marker = code.find("//")
        if marker != -1:
            code = code[:marker]
        out.append((number, code))
    return out


def main() -> int:
    root = Path(__file__).resolve().parent
    leaks: list[str] = []
    scanned = 0

    for crate in NEUTRAL_CRATES:
        crate_dir = root / crate
        if not crate_dir.is_dir():
            print(f"warning: {crate} not found", file=sys.stderr)
            continue
        for path in sorted(crate_dir.rglob("*.rs")):
            scanned += 1
            source = path.read_text(encoding="utf-8")
            for number, code in strip_comments(source):
                match = PATTERN.search(code)
                if match:
                    rel = path.relative_to(root)
                    leaks.append(f"{rel}:{number}: '{match.group(0)}' in {code.strip()}")

    if leaks:
        print(f"Domain vocabulary leaked into neutral crates ({len(leaks)}):\n")
        for leak in leaks:
            print(f"  {leak}")
        print(
            "\nMove it to a domain profile crate, or rename it to the neutral "
            "concept it represents."
        )
        return 1

    print(f"neutrality: ok ({scanned} files scanned, no leaks)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
