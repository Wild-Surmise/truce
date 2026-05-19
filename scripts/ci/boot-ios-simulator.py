#!/usr/bin/env python3
"""Validate the pinned iOS sim device + runtime are present on this
GHA runner, then create + boot a simulator under the name passed
as the first positional arg.

Used by `.github/workflows/ci-ios.yml` (sim name `truce-ci`) and
`.github/workflows/bake-screenshots.yml` (sim name `truce-bake`)
so the device + runtime pin lives in one place instead of two
diverging copies.

Required env vars (set at the workflow level):
  IOS_SIM_DEVICE   - SimDeviceType identifier
                     (e.g. com.apple.CoreSimulator.SimDeviceType.iPhone-17-Pro)
  IOS_SIM_RUNTIME  - SimRuntime identifier
                     (e.g. com.apple.CoreSimulator.SimRuntime.iOS-26-3)

Usage:  python3 scripts/ci/boot-ios-simulator.py <sim-name>

On failure (pinned device or runtime not available on the runner
image), prints the available IDs to stderr so the next workflow
bump is a one-line edit instead of guessing.
"""

import json
import os
import subprocess
import sys


def main() -> int:
    if len(sys.argv) != 2:
        print(f"usage: {sys.argv[0]} <sim-name>", file=sys.stderr)
        return 2
    sim_name = sys.argv[1]

    want_dev = os.environ["IOS_SIM_DEVICE"]
    want_rt = os.environ["IOS_SIM_RUNTIME"]
    devs = json.loads(
        subprocess.check_output(["xcrun", "simctl", "list", "devicetypes", "-j"])
    )["devicetypes"]
    rts = json.loads(
        subprocess.check_output(["xcrun", "simctl", "list", "runtimes", "-j"])
    )["runtimes"]

    if not any(d["identifier"] == want_dev for d in devs):
        print(f"::error::IOS_SIM_DEVICE not available: {want_dev}", file=sys.stderr)
        print("Available iPhone Pro device types on this runner:", file=sys.stderr)
        for d in devs:
            if "iPhone" in d["name"] and "Pro" in d["name"] and "Max" not in d["name"]:
                print(f"  {d['identifier']}", file=sys.stderr)
        return 1

    if not any(r["identifier"] == want_rt and r["isAvailable"] for r in rts):
        print(f"::error::IOS_SIM_RUNTIME not available: {want_rt}", file=sys.stderr)
        print("Available iOS runtimes on this runner:", file=sys.stderr)
        for r in rts:
            if r["isAvailable"] and r["identifier"].startswith(
                "com.apple.CoreSimulator.SimRuntime.iOS-"
            ):
                print(f"  {r['identifier']}", file=sys.stderr)
        return 1

    subprocess.check_call(["xcrun", "simctl", "create", sim_name, want_dev, want_rt])
    subprocess.check_call(["xcrun", "simctl", "boot", sim_name])
    subprocess.check_call(["xcrun", "simctl", "bootstatus", sim_name])
    return 0


if __name__ == "__main__":
    sys.exit(main())
