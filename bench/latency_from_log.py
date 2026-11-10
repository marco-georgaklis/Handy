#!/usr/bin/env python3
"""Dictation latency from the app's own log.

Reads the `dictation timing:` line this fork writes once per dictation
(key release -> text pasted, split into transcribe / post-process / paste)
and prints percentiles. Logs from upstream builds have no such line, so for
those it falls back to the per-step debug lines (needs log level Debug).

  python3 bench/latency_from_log.py                # the installed app's log
  python3 bench/latency_from_log.py path/to/handy.log --since 2026-11-10
"""
import argparse
import pathlib
import re
import statistics

DEFAULT_LOG = pathlib.Path.home() / "Library/Logs/com.handyflow.app/handy.log"
TIMING = re.compile(
    r"dictation timing: audio=([\d.]+)s transcribe=(\d+)ms post_process=(\d+)ms"
    r" paste=(\d+)ms total=(\d+)ms"
)
DURATION = re.compile(r"in ([\d.]+)(ms|µs|s)\b")
UNIT_MS = {"ms": 1.0, "µs": 0.001, "s": 1000.0}


def duration_ms(line):
    match = DURATION.search(line)
    return float(match[1]) * UNIT_MS[match[2]] if match else None


def percentiles(name, values, unit="ms"):
    if not values:
        return
    ordered = sorted(values)
    p = lambda q: ordered[min(len(ordered) - 1, int(len(ordered) * q))]
    print(
        f"  {name:<22} n={len(ordered):<4} p50={statistics.median(ordered):8.0f}{unit}"
        f"  p90={p(0.9):8.0f}{unit}  p95={p(0.95):8.0f}{unit}  max={ordered[-1]:8.0f}{unit}"
    )


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("log", nargs="?", default=str(DEFAULT_LOG))
    parser.add_argument("--since", help="only lines dated YYYY-MM-DD or later (log dates are UTC)")
    args = parser.parse_args()

    lines = []
    for line in open(args.log, errors="replace"):
        if args.since and line[1:11] < args.since:
            continue
        lines.append(line)

    rows = [tuple(float(x) for x in m.groups()) for m in map(TIMING.search, lines) if m]
    if rows:
        print(f"{len(rows)} dictations (key release -> pasted)")
        percentiles("total", [r[4] for r in rows])
        percentiles("transcribe", [r[1] for r in rows])
        percentiles("post-process (all)", [r[2] for r in rows])
        percentiles("post-process (ran)", [r[2] for r in rows if r[2] > 5])
        percentiles("paste", [r[3] for r in rows])
        skipped = sum(r[2] <= 5 for r in rows)
        print(f"  post-processing skipped or off in {skipped}/{len(rows)}")
        percentiles("audio length", [r[0] * 1000 for r in rows])
        return

    # Upstream log: sum the steps that happen after key release.
    stop = [duration_ms(l) for l in lines if "samples retrieved in" in l]
    transcribe = [duration_ms(l) for l in lines if "actions][DEBUG] Transcription completed in" in l]
    paste = [duration_ms(l) for l in lines if "Text pasted successfully in" in l]
    mic = [duration_ms(l) for l in lines if "Microphone stream initialized in" in l]
    n = min(len(stop), len(transcribe), len(paste))
    if not n:
        print("No dictations found (is the log level Debug?)")
        return
    print(f"{n} dictations, upstream log format (no post-processing time recorded)")
    percentiles("total (sum of steps)", [stop[i] + transcribe[i] + paste[i] for i in range(n)])
    percentiles("stop recording", stop)
    percentiles("transcribe", transcribe)
    percentiles("paste", paste)
    percentiles("mic open (key down)", mic)


if __name__ == "__main__":
    main()
