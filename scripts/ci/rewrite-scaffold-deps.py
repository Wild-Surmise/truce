#!/usr/bin/env python3
"""Rewrite scaffolded `{ git = "...truce", tag = "..." }` Cargo.toml
dependencies to `{ path = "<workspace>/crates/<name>" }` so that a
scaffolded project consumes truce HEAD from the checked-out
workspace instead of the published tag the scaffold template
generated against.

Used by `.github/workflows/cli-scaffold.yml` (scaffold + build with
current cargo-truce) and `.github/workflows/cli-backcompat.yml`
(scaffold with the LAST released cargo-truce, then build against
truce HEAD).

Required env vars:
  GITHUB_WORKSPACE - the truce checkout (provides crates/* paths)
  SCAFFOLD_DIR     - absolute path to the scaffolded project root

Exits non-zero if no Cargo.toml in the scaffold tree contained the
expected git dep needle - that's a signal the scaffold template
layout drifted and the script's pattern needs updating to match.
"""

import os
import pathlib
import re
import sys


def main() -> int:
    truce_crates = pathlib.Path(os.environ["GITHUB_WORKSPACE"], "crates").as_posix()
    root = pathlib.Path(os.environ["SCAFFOLD_DIR"])
    needle = '{ git = "https://github.com/truce-audio/truce"'
    tag_re = re.compile(r',\s*tag\s*=\s*"[^"]*"')

    rewrote = 0
    for path in root.rglob("Cargo.toml"):
        content = path.read_text()
        if needle not in content:
            continue
        new_lines = []
        for line in content.splitlines():
            stripped = line.lstrip()
            if stripped.startswith("#") or needle not in line:
                new_lines.append(line)
                continue
            eq = line.find("=")
            key = line[:eq].strip() if eq >= 0 else "truce"
            replacement = f'{{ path = "{truce_crates}/{key}"'
            out = line.replace(needle, replacement, 1)
            out = tag_re.sub("", out, count=1)
            new_lines.append(out)
        suffix = "\n" if content.endswith("\n") else ""
        path.write_text("\n".join(new_lines) + suffix)
        rewrote += 1
        print(f"rewrote {path}")

    if rewrote == 0:
        print(
            "::error::no Cargo.toml had a git dep to rewrite - "
            "scaffold layout may have changed",
            file=sys.stderr,
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
