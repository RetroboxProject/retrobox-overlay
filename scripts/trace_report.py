#!/usr/bin/env python3
"""Summarize an overlay trace (RETROBOX_OVERLAY_TRACE), with a probe's
samples of the system (probe.py) when given, to see where the time goes.

It reads a session of the trace (the last one by default): the drags
(a finger or a button held, from down to up) and, outside them, the rest
of the time the menu is open; for each, how many input events came and how
late they were handled, how many frames were drawn and what each cost (to
draw, to fill the buffer, to hash its rows, to commit), how long the
compositor took to show them (the frame callbacks) and how regularly it
refreshes; and, from the probe, the CPU use, the waits for a core and the
preemptions of the game's, the compositor's and the overlay's threads, the
cores' use and frequency and the GPU's load, then and before the overlay
opened (the game alone).

    scripts/trace_report.py /tmp/trace.log --probe /tmp/probe.log
"""

import argparse
import statistics
import sys
from collections import defaultdict

INPUTS = ("touch_motion", "pointer_motion")
"""Events that move what is held."""
DOWNS = ("touch_down", "pointer_press")
UPS = ("touch_up", "pointer_release")
BASELINE_MS = 5000.0
"""How much of the probe before the overlay opened is the game alone."""


def parse(path):
    """The lines of a trace or a probe: [(time, event, {field: value})]."""
    lines = []
    with open(path, encoding="utf-8") as text:
        for line in text:
            if line.startswith("#") or not line.strip():
                continue
            time_text, event, *pairs = line.split()
            fields = dict(pair.split("=", 1) for pair in pairs if "=" in pair)
            lines.append((float(time_text), event, fields))
    return lines


def header(path):
    """A probe's header lines."""
    with open(path, encoding="utf-8") as text:
        return [line.rstrip() for line in text if line.startswith("#")]


def sessions(lines):
    """The trace split at each `start`."""
    found = []
    for line in lines:
        if line[1] == "start" or not found:
            found.append([])
        found[-1].append(line)
    return found


def numbers(lines, event, field, **match):
    """The values of a field of these events (that have these fields)."""
    values = []
    for _, name, fields in lines:
        if name == event and all(fields.get(key) == value for key, value in match.items()):
            try:
                values.append(float(fields[field]))
            except (KeyError, ValueError):
                pass
    return values


def spread(values, unit="ms"):
    """`n, median, 95th percentile, max` of some values."""
    if not values:
        return "-"
    ordered = sorted(values)
    high = ordered[min(len(ordered) - 1, int(len(ordered) * 0.95))]
    return (
        f"n={len(values)} median={statistics.median(ordered):.2f}{unit} "
        f"p95={high:.2f}{unit} max={ordered[-1]:.2f}{unit}"
    )


def intervals(times):
    """The time between consecutive ones."""
    return [later - earlier for earlier, later in zip(times, times[1:])]


def drags(lines):
    """The windows (start, end) from a press to its release."""
    windows = []
    start = None
    for time, event, _ in lines:
        if event in DOWNS and start is None:
            start = time
        elif event in UPS and start is not None:
            windows.append((start, time))
            start = None
    return windows


def within(lines, windows):
    """The lines inside any of these windows."""
    return [line for line in lines if any(start <= line[0] <= end for start, end in windows)]


def outside(lines, windows, span):
    """The lines in `span` but in none of the windows."""
    first, last = span
    return [
        line
        for line in lines
        if first <= line[0] <= last and not any(start <= line[0] <= end for start, end in windows)
    ]


def report_overlay(title, lines, seconds):
    """What the overlay did in these lines."""
    print(f"\n== {title}: {seconds:.1f} s")
    if seconds <= 0:
        return
    inputs = [line for line in lines if line[1] in INPUTS]
    print(f"input: {len(inputs) / seconds:.0f}/s")
    print(f"  handled late by: {spread(numbers(lines, 'touch_motion', 'lag'))} (touch)")
    print(f"                   {spread(numbers(lines, 'pointer_motion', 'lag'))} (pointer)")
    print(f"  between events:  {spread(intervals([line[0] for line in inputs]))}")
    steps = [line for line in lines if line[1] == "scroll"]
    scrolled = sum(1 for line in steps if line[2].get("scrolled") == "true")
    print(f"scroll steps: {len(steps)}, {scrolled} moved the list")
    draws = [line for line in lines if line[1] == "draw" and line[2].get("piece") == "panel"]
    defers = sum(1 for line in lines if line[1] == "defer" and line[2].get("piece") == "panel")
    timeouts = sum(1 for line in lines if line[1] == "timeout" and line[2].get("piece") == "panel")
    print(
        f"panel frames: {len(draws) / seconds:.1f}/s ({len(draws)}), "
        f"{defers} put off, {timeouts} drawn by the timer (no frame callback)"
    )
    print(f"  render:  {spread(numbers(lines, 'draw', 'render', piece='panel'))}")
    print(f"  present: {spread(numbers(lines, 'draw', 'present', piece='panel'))}")
    for field in ("create", "fill", "hash"):
        print(f"    {field:7}{spread(numbers(lines, 'buffer', field))}")
    print(f"    commit {spread(numbers(lines, 'commit', 'took'))}")
    print(f"  between frames: {spread(intervals([line[0] for line in draws]))}")
    waited = numbers(lines, "frame_done", "waited", piece="panel")
    waited = [value for value in waited if value >= 0]
    print(f"frame callbacks (commit to done): {spread(waited)}")
    times = numbers(lines, "frame_done", "time", piece="panel")
    print(f"  compositor's frame times apart: {spread(intervals(times))}")
    print(f"  delivered late by: {spread(numbers(lines, 'frame_done', 'ago', piece='panel'))}")
    gaps = sorted(intervals([line[0] for line in lines]), reverse=True)[:5]
    print(f"longest gaps between events: {', '.join(f'{gap:.1f}ms' for gap in gaps) or '-'}")


def thread_table(samples):
    """{(process, thread name, tid): [samples' fields]}."""
    threads = defaultdict(list)
    for _, event, fields in samples:
        if event == "thread":
            threads[(fields["proc"], fields["name"], fields["tid"])].append(fields)
    return threads


def report_threads(samples, seconds, count):
    """Each process's and the busiest threads' CPU use, waits for a core
    and preemptions."""
    threads = thread_table(samples)
    processes = defaultdict(lambda: [0.0, 0.0, 0])
    for (process, _, _), items in threads.items():
        totals = processes[process]
        totals[0] += sum(float(fields["cpu"]) for fields in items) / count
        totals[1] += sum(float(fields["wait"]) for fields in items) / seconds
        totals[2] += sum(int(fields["preempted"]) for fields in items) / seconds
    print("process             cpu%  run-queue wait ms/s  preempted/s")
    for process, (cpu, wait, preempted) in sorted(processes.items(), key=lambda item: -item[1][0]):
        print(f"{process:18} {cpu:5.1f} {wait:20.1f} {preempted:12.1f}")
    print("busiest threads              cpu%  wait ms/s  cores")
    ranked = sorted(threads.items(), key=lambda item: -sum(float(f["cpu"]) for f in item[1]))
    for (process, name, tid), items in ranked[:12]:
        cpu = sum(float(fields["cpu"]) for fields in items) / count
        wait = sum(float(fields["wait"]) for fields in items) / seconds
        cores = ",".join(sorted({fields["core"] for fields in items}, key=int))
        print(f"{process[:9]:9} {name[:12]:12} {tid:>6} {cpu:5.1f} {wait:10.1f}  {cores}")


def report_devices(samples):
    """The cores' use and frequency, and the GPU's load."""
    cores = defaultdict(list)
    freqs = defaultdict(list)
    for _, event, fields in samples:
        if event == "core":
            cores[fields["core"]].append(float(fields["use"]))
            if fields["khz"] != "-":
                freqs[fields["core"]].append(int(fields["khz"]) // 1000)
    for core in sorted(cores, key=int):
        frequency = f" at {min(freqs[core])}-{max(freqs[core])} MHz" if freqs[core] else ""
        print(f"core {core}: {statistics.mean(cores[core]):5.1f}% busy{frequency}")
    gpu = [fields for _, event, fields in samples if event == "gpu"]
    loads = [float(fields["load"]) / 10.0 for fields in gpu if fields["load"] != "-"]
    if loads:
        hertz = "/".join(sorted({fields["hz"] for fields in gpu}))
        emc = "/".join(sorted({fields["emc"] for fields in gpu}))
        print(
            f"gpu: {statistics.mean(loads):.1f}% load (max {max(loads):.1f}%), "
            f"freq {hertz}, emc {emc}"
        )


def report_probe(title, samples):
    """What the system did in these samples."""
    print(f"\n-- system, {title}")
    times = sorted({time for time, event, _ in samples if event == "core"})
    if len(times) < 2:
        print("no samples")
        return
    count = len(times)
    seconds = count * statistics.mean(intervals(times)) / 1000.0
    report_threads(samples, seconds, count)
    report_devices(samples)


def main():
    """Read the files and print the report."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n", maxsplit=1)[0])
    parser.add_argument("trace", help="the overlay's trace")
    parser.add_argument("--probe", help="probe.py's samples, taken at the same time")
    parser.add_argument("--session", type=int, default=-1, help="which session (default: last)")
    args = parser.parse_args()
    found = sessions(parse(args.trace))
    if not found:
        sys.exit("the trace is empty")
    session = found[args.session]
    first, last = session[0][0], session[-1][0]
    windows = drags(session)
    dragging = sum(end - start for start, end in windows) / 1000.0
    print(f"session {args.session % len(found) + 1} of {len(found)}: {(last - first) / 1000:.1f} s")
    print(f"{len(windows)} drags, {dragging:.1f} s in all")
    report_overlay("while dragging", within(session, windows), dragging)
    rest = outside(session, windows, (first, last))
    report_overlay("open, not dragging", rest, (last - first) / 1000.0 - dragging)
    if not args.probe:
        return
    for line in header(args.probe):
        print(line)
    samples = parse(args.probe)
    report_probe("the game alone (before the overlay)", [
        sample for sample in samples if first - BASELINE_MS <= sample[0] < first
    ])
    report_probe("while dragging", within(samples, windows))
    report_probe("open, not dragging", outside(samples, windows, (first, last)))


if __name__ == "__main__":
    main()
