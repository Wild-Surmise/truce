#!/usr/bin/env python3
"""Exercise the native AU state restoration and document-change notifications.

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
    with (tmp / "BridgingHeader.h").open("a") as header:
        header.write("\nvoid mock_mark_state_dirty(void);\nvoid mock_use_previous_abi(void);\n")
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
                              "rejected-legacy", "nil", "dirty", "dirty-previous-abi",
                              "dirty-v2", "dirty-v2-previous-abi",
                              "dirty-v2-queued-close", "dirty-v2-reentrant-close"]
    if any(case.startswith("dirty-v2") for case in cases):
        subprocess.run(["xcrun", "clang", "-I", str(tmp),
                        str(crate.parent / "truce-au/shim/au_v2_shim.c"),
                        str(here / "au2_dirty.c"), str(tmp / "mock.o"),
                        "-framework", "AudioToolbox", "-framework", "CoreFoundation",
                        "-framework", "CoreMIDI", "-o", str(tmp / "au2_probe")], check=True)
    for case in cases:
        if case.startswith("dirty-v2"):
            command = [str(tmp / "au2_probe")]
            if case.startswith("dirty-v2-"):
                command.append(case.removeprefix("dirty-v2-"))
        else:
            command = [str(tmp / "probe"), case]
        result = subprocess.run(command, capture_output=True,
                                text=True, timeout=10)
        assert result.returncode == 0, f"{case}: {result.returncode}\n{result.stdout}\n{result.stderr}"
        print(result.stdout.strip(), flush=True)
