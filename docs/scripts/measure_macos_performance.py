#!/usr/bin/env python3
"""Measure the real Metal UI with isolated preferences (Python standard library only)."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import subprocess
import sys
import time


def cpu_seconds(value):
    seconds = 0.0
    for component in value.split(":"):
        seconds = seconds * 60 + float(component)
    return seconds


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, default=Path("target/release/arkpets-ui"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--page", choices=("library", "settings"), default="library")
    parser.add_argument("--model", default="pepe")
    parser.add_argument("--fps", type=int, default=60)
    parser.add_argument("--warmup", type=int, default=3)
    parser.add_argument("--duration", type=int, default=12)
    args = parser.parse_args()
    if sys.platform != "darwin":
        parser.error("this script requires a logged-in macOS graphical desktop")
    if not 1 <= args.fps <= 240 or not 2 <= args.warmup <= 60 or not 8 <= args.duration <= 120:
        parser.error("fps must be 1..240, warmup 2..60, and duration 8..120")
    executable = args.executable.resolve(strict=True)
    executable_hash = hashlib.sha256(executable.read_bytes()).hexdigest()
    output = args.output or Path("target/macos-performance") / (
        time.strftime("%Y%m%d-%H%M%S") + f"-{args.page}-{time.time_ns() % 1_000_000_000}"
    )
    output = output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    env = os.environ.copy()
    env.pop("ARKPETS_CAPTURE_DIR", None)
    env.update(
        ARKPETS_RENDERER="metal",
        ARKPETS_ASSETS=str(Path(__file__).resolve().parents[2] / "assets"),
        ARKPETS_PROFILE_DIR=str(output),
        ARKPETS_PROFILE_PAGE=args.page,
        ARKPETS_PROFILE_MODEL=args.model,
        ARKPETS_PROFILE_FPS=str(args.fps),
        ARKPETS_PROFILE_WARMUP=str(args.warmup),
        ARKPETS_PROFILE_SECONDS=str(args.duration),
    )
    print(f"Measuring {args.model}, {args.page}, {args.fps} Hz: {output}", flush=True)
    samples = []
    with (output / "stdout.log").open("w") as stdout, (output / "stderr.log").open("w") as stderr:
        process = subprocess.Popen([str(executable)], env=env, stdout=stdout, stderr=stderr)
        deadline = time.monotonic() + args.warmup + args.duration + 30
        try:
            while process.poll() is None:
                if time.monotonic() >= deadline:
                    raise TimeoutError(f"UI did not finish; inspect {output / 'stderr.log'}")
                started = time.time_ns()
                result = subprocess.run(
                    ["/bin/ps", "-p", str(process.pid), "-o", "time=,rss="],
                    capture_output=True, text=True, check=False,
                )
                finished = time.time_ns()
                if result.returncode != 0:
                    if process.poll() is not None:
                        break
                    raise RuntimeError(f"ps failed: {result.stderr.strip()}")
                cpu, rss = result.stdout.split()
                samples.append({
                    "unix_ms": (started + finished) // 2_000_000,
                    "cpu_seconds": cpu_seconds(cpu),
                    "resident_bytes": int(rss) * 1024,
                })
                time.sleep(0.1)
            if process.returncode != 0:
                raise RuntimeError(f"UI exited with {process.returncode}; inspect {output / 'stderr.log'}")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()

    frames = json.loads((output / "frames.json").read_text())
    if frames["debug_assertions"]:
        raise RuntimeError("Use a release executable for performance comparisons")
    if frames["settings"]["page"] != args.page:
        raise RuntimeError("Executable did not run the requested page")
    measured = [sample for sample in samples if
                frames["measurement_start_unix_ms"] <= sample["unix_ms"] <= frames["measurement_end_unix_ms"]]
    if len(measured) < 2:
        raise RuntimeError("Too few process samples within the measurement window")
    first, last = measured[0], measured[-1]
    elapsed = (last["unix_ms"] - first["unix_ms"]) / 1000
    cpu = (last["cpu_seconds"] - first["cpu_seconds"]) / elapsed * 100
    resident = [sample["resident_bytes"] for sample in measured]
    report = {
        "version": 1,
        "os": platform.platform(),
        "executable": str(executable),
        "sha256": executable_hash,
        "process_id": process.pid,
        "metric_scope": "ps cumulative process CPU time and resident memory, sampled about every 100 ms. "
                        "CPU 100% is one logical core. RSS is not GPU memory. No display-present FPS is measured.",
        "measurement_start_unix_ms": first["unix_ms"],
        "measurement_end_unix_ms": last["unix_ms"],
        "cpu_percent_one_core": cpu,
        "resident_mean_bytes": sum(resident) / len(resident),
        "resident_peak_bytes": max(resident),
        "samples": measured,
    }
    (output / "process.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({
        "cpu_percent_one_core": round(cpu, 2),
        "resident_mean_mib": round(report["resident_mean_bytes"] / 1024**2, 2),
        "root": frames["root"],
        "desktop": frames["desktop"],
        "animation_update_rate_hz": frames.get("animation_update_rate_hz"),
    }, indent=2))


if __name__ == "__main__":
    main()
