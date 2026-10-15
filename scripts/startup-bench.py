#!/usr/bin/env python3
"""Launch timings: runs BenCode N times with BENCODE_STARTUP_TRACE=quit on a
scratch copy of the data folder and prints the median of each step.

    scripts/startup-bench.py [--runs N] [--heavy] target/release/bencode [other/bencode ...]

`--heavy` gives each of the 50 threads a launch loads the transcript of the
longest one, as a long history would.

With several binaries the runs alternate between them, so a machine that
gets busier or quieter halfway weighs on each the same.

`exec→` columns are wall-clock from this script's exec of the binary, so
they include loading the binary itself, which the in-process marks (from
`main`) cannot see. The first run warms the disk cache and is dropped.
"""

import os
import re
import shutil
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time

LINE = re.compile(r"startup\s+([\d.]+)ms\s+cpu\s+([\d.]+)ms\s+(.*)")


def scratch_home(heavy=False):
    home = tempfile.mkdtemp(prefix="bencode-startup-")
    data = os.path.join(home, "Library", "Application Support", "BenCode")
    src = os.path.expanduser("~/Library/Application Support/BenCode")
    os.makedirs(data)
    # A consistent copy of the live database, its WAL folded in.
    live = sqlite3.connect(f"file:{src}/bencode.db?mode=ro", uri=True)
    copy = sqlite3.connect(os.path.join(data, "bencode.db"))
    live.backup(copy)
    if heavy:
        copy.execute(
            "UPDATE sessions SET blocks_json = (SELECT blocks_json FROM sessions "
            "ORDER BY length(blocks_json) DESC LIMIT 1) WHERE id IN (SELECT id FROM "
            "sessions WHERE inbox_ask IS NULL ORDER BY updated_at DESC LIMIT 50)"
        )
        copy.commit()
    copy.close()
    live.close()
    for name in ("settings.json",):
        shutil.copy(os.path.join(src, name), data)
    for name in ("checkpoints", "monos", "backgrounds"):
        if os.path.isdir(os.path.join(src, name)):
            shutil.copytree(os.path.join(src, name), os.path.join(data, name))
    return home


def run_once(binary, home):
    env = dict(os.environ, HOME=home, BENCODE_STARTUP_TRACE="quit")
    marks = {}
    started = time.perf_counter()
    proc = subprocess.Popen(
        [binary], env=env, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True
    )
    for line in proc.stdout:
        m = LINE.match(line.strip())
        if m:
            step = m.group(3)
            marks[step] = (float(m.group(1)), float(m.group(2)))
            if step == "first frame presented":
                marks["exec→first frame"] = ((time.perf_counter() - started) * 1000, None)
            if step == "launch settled":
                marks["exec→settled"] = ((time.perf_counter() - started) * 1000, None)
    try:
        proc.wait(timeout=20)
    except subprocess.TimeoutExpired:
        proc.kill()
    return marks


def main():
    args = sys.argv[1:]
    runs = 10
    heavy = False
    while args and args[0].startswith("--"):
        if args[0] == "--runs":
            runs = int(args[1])
            args = args[2:]
        elif args[0] == "--heavy":
            heavy = True
            args = args[1:]
        else:
            sys.exit(f"unknown option {args[0]}")
    binaries = [os.path.abspath(b) for b in args]
    home = scratch_home(heavy)
    results = {b: [] for b in binaries}
    try:
        for b in binaries:
            run_once(b, home)  # warms the disk cache; not counted
        for _ in range(runs):
            for b in binaries:
                results[b].append(run_once(b, home))
    finally:
        # Shells the dock restored run in a host of the scratch folder.
        for b in binaries:
            subprocess.run(["pkill", "-f", f"{b} --pty-"], check=False)
        shutil.rmtree(home, ignore_errors=True)
    for b in binaries:
        report(b, results[b], runs)


def report(binary, results, runs):
    order = []
    for r in results:
        order += [step for step in r if step not in order]
    print(f"\n{binary}")
    print(f"{'step':<44} {'median':>9} {'min':>9} {'max':>9} {'cpu':>9}   ({runs} runs)")
    for step in order:
        values = [r[step][0] for r in results if step in r]
        cpus = [r[step][1] for r in results if step in r and r[step][1] is not None]
        cpu = f"{statistics.median(cpus):>7.1f}ms" if cpus else ""
        print(
            f"{step:<44} {statistics.median(values):>7.1f}ms "
            f"{min(values):>7.1f}ms {max(values):>7.1f}ms {cpu:>9}"
        )


if __name__ == "__main__":
    main()
