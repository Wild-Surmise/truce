#!/usr/bin/env python3
"""Compile the shipping AUv3 template and exercise native state restoration.

Requires macOS + Xcode. No installed plugin, simulator, signing or DSP needed.
Each scenario is a separate process: an Objective-C exception is a test failure.
"""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

here = Path(__file__).resolve().parent
crate = here.parent.parent
with tempfile.TemporaryDirectory(prefix="truce-au3-state-") as tmp:
    tmp = Path(tmp)
    for name in ["AudioUnitFactory.swift", "BridgingHeader.h"]:
        shutil.copy(crate / "templates/au3" / name, tmp / name)
    shutil.copy(crate.parent / "truce-shim-types/include/au_shim_types.h", tmp)
    shutil.copy(here / "main.swift", tmp)
    subprocess.run(["xcrun", "clang", "-c", str(here / "mock_plugin.c"),
                    "-I", str(tmp), "-o", str(tmp / "mock.o")], check=True)
    subprocess.run(["xcrun", "swiftc", "-O", "-import-objc-header", str(tmp / "BridgingHeader.h"),
                    str(tmp / "AudioUnitFactory.swift"), str(tmp / "main.swift"),
                    str(tmp / "mock.o"), "-o", str(tmp / "probe")], check=True)
    cases = sys.argv[1:] or ["roundtrip", "native", "legacy", "apple-only",
                              "invalid-apple-only", "invalid-type", "partial",
                              "invalid-type-apple-only", "empty-apple-only",
                              "rejected-legacy", "nil"]
    for case in cases:
        result = subprocess.run([str(tmp / "probe"), case], capture_output=True,
                                text=True, timeout=10)
        assert result.returncode == 0, f"{case}: {result.returncode}\n{result.stdout}\n{result.stderr}"
        print(result.stdout.strip(), flush=True)
