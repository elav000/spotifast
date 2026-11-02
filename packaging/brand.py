#!/usr/bin/env python3
"""Read the shared product identity manifest for packaging scripts."""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: brand.py FIELD")
    identity = tomllib.loads((Path(__file__).parents[1] / "brand.toml").read_text())
    try:
        print(identity[sys.argv[1]])
    except KeyError as error:
        raise SystemExit(f"unknown brand field: {sys.argv[1]}") from error
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
