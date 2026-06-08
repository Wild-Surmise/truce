#!/usr/bin/env python3
#
# topo.py — emit truce workspace crates in publish order, one per line.
#
# Filter: workspace members under `crates/` (examples are scaffolded
# demonstrations, not published, and crates that opt out via
# `publish = false` in their Cargo.toml are skipped). Topo-sort by
# intra-workspace dependencies so every dep is on the registry before
# its dependents.
#
# Invoked by release.sh; runnable standalone for debugging the order.

import json
import os
import shutil
import subprocess
import sys


def cargo_bin():
    # Honor an explicit CARGO from the caller (release.sh). Otherwise
    # prefer cargo.exe — on Windows (WSL) both cargo.exe and cargo can
    # be on PATH — and fall back to cargo.
    explicit = os.environ.get("CARGO")
    if explicit:
        return explicit
    for cand in ("cargo.exe", "cargo"):
        if shutil.which(cand):
            return cand
    return "cargo"


meta = json.loads(subprocess.check_output(
    [cargo_bin(), "metadata", "--format-version", "1", "--no-deps"]))

ws_members = set(meta["workspace_members"])
pkgs = [
    p for p in meta["packages"]
    if p["id"] in ws_members
    and "/crates/" in p["manifest_path"]
    and p.get("publish") != []  # opt-out via `publish = false`
]
names = {p["name"] for p in pkgs}

# Edges: package -> set of intra-workspace dep names. A dep listed
# multiple times (e.g. once per target / per kind) collapses naturally
# through the set. Dev-deps (`kind == "dev"`) are stripped at publish
# time, so excluding them here matches the actual on-registry shape.
# The current workspace is cycle-free in normal deps (test crates
# that need a `truce` dev-dep live in their own publish=false crate,
# `truce-loader-tests`), but the filter is kept defensively: a
# future test relocation that re-introduces a back-edge shouldn't
# silently break the publish topology.
incoming = {p["name"]: set() for p in pkgs}
for p in pkgs:
    for d in p["dependencies"]:
        if d.get("kind") == "dev":
            continue
        if d["name"] in names and d["name"] != p["name"]:
            incoming[p["name"]].add(d["name"])

# Kahn topo sort, alphabetical tie-break for determinism.
order = []
ready = sorted(n for n, deps in incoming.items() if not deps)
while ready:
    n = ready.pop(0)
    order.append(n)
    for m, deps in list(incoming.items()):
        if n in deps:
            deps.discard(n)
            if not deps and m not in order and m not in ready:
                ready.append(m)
    ready.sort()

remaining = [n for n in incoming if n not in order]
if remaining:
    sys.exit(f"cycle: unresolved={remaining}")

# Force the ordering of user-selectable crates
forced_order = [
    "truce-simd",
    "truce-vst2",
    "truce-lv2",
    "truce-aax",
    "truce-au",
    "truce-standalone",
    "truce-clap",
    "truce-vst3",
    "truce",
    "cargo-truce",
]

missing_forced = [name for name in forced_order if name not in order]
if missing_forced:
    sys.exit(
        "forced ordering crates missing from publish order: "
        + ", ".join(missing_forced)
    )

order = [name for name in order if name not in forced_order] + forced_order

print("\n".join(order))
