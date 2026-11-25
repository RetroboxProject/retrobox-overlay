#!/usr/bin/env python3
"""Sample what the machine does while the overlay runs, to diagnose it.

Every INTERVAL seconds it writes, for each thread of the processes it
watches (the game, the compositor, the overlay and a few others), its CPU
use, how long it waited for a core (the run queue: a thread that is ready
but not running) and how often it was preempted; for each core its use and
frequency; and the GPU's load and frequency, the memory controller's
frequency and the temperatures, where the kernel tells them (Tegra's L4T
among others). The header has what the compositor renders with.

Lines are `<time> <event> <field>=<value>...`, the time being
CLOCK_MONOTONIC in milliseconds, as in the overlay's trace
(RETROBOX_OVERLAY_TRACE), so that `trace_report.py` can put both
together:

    scripts/probe.py --out /tmp/probe.log        # Ctrl+C to stop
    scripts/trace_report.py /tmp/trace.log --probe /tmp/probe.log
"""

import argparse
import os
import platform
import re
import sys
import time
from pathlib import Path

WATCHED = (
    "retroarch",
    "labwc",
    "retrobox-overla",
    "retrobox-inputd",
    "emulationstatio",
    "pulseaudio",
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "Xwayland",
)
"""Process names (as /proc/PID/comm has them: 15 characters at most)."""

GPU_LOADS = (
    "/sys/devices/gpu.0/load",
    "/sys/devices/57000000.gpu/load",
    "/sys/devices/platform/gpu.0/load",
)
GPU_FREQS = (
    "/sys/devices/57000000.gpu/devfreq/57000000.gpu/cur_freq",
    "/sys/devices/gpu.0/devfreq/57000000.gpu/cur_freq",
    "/sys/kernel/debug/clk/gbus/clk_rate",
)
EMC_FREQS = (
    "/sys/kernel/actmon_avg_activity/emc_freq",
    "/sys/kernel/debug/clk/emc/clk_rate",
    "/sys/kernel/debug/bpmp/debug/clk/emc/rate",
)
RENDERER_LIBRARIES = re.compile(
    r"lib(GLESv2|EGL|EGL_nvidia|nvidia-[a-z]+|vulkan|pixman-1|gbm|drm)[^/]*\.so"
)
TICKS = os.sysconf("SC_CLK_TCK")


def now():
    """CLOCK_MONOTONIC, in milliseconds."""
    return time.monotonic() * 1000.0


def read(path):
    """A small file's text, stripped (None when it cannot be read)."""
    try:
        return Path(path).read_text(encoding="utf-8", errors="replace").strip()
    except OSError:
        return None


def first_readable(paths):
    """The first of these files that can be read, and its text."""
    for path in paths:
        text = read(path)
        if text:
            return path, text
    return None, None


def emit(out, event, **fields):
    """Write a line."""
    text = " ".join(f"{key}={value}" for key, value in fields.items())
    out.write(f"{now():.3f} {event} {text}\n")


def watched_processes():
    """The watched processes running: {pid: name}."""
    found = {}
    for entry in os.listdir("/proc"):
        if entry.isdigit():
            name = read(f"/proc/{entry}/comm")
            if name in WATCHED:
                found[int(entry)] = name
    return found


def thread_sample(pid, tid):
    """A thread's counters: name, CPU ticks, run-queue wait (ns),
    involuntary switches, last core (None when it is gone)."""
    stat = read(f"/proc/{pid}/task/{tid}/stat")
    if not stat:
        return None
    # the name is in parentheses, and may hold spaces
    name = stat[stat.index("(") + 1 : stat.rindex(")")]
    fields = stat[stat.rindex(")") + 2 :].split()
    # fields[0] is field 3 of proc(5): utime is 14, stime 15, processor 39
    ticks = int(fields[11]) + int(fields[12])
    core = int(fields[36])
    schedstat = read(f"/proc/{pid}/task/{tid}/schedstat")
    wait = int(schedstat.split()[1]) if schedstat else 0
    status = read(f"/proc/{pid}/task/{tid}/status") or ""
    match = re.search(r"nonvoluntary_ctxt_switches:\s+(\d+)", status)
    preempted = int(match.group(1)) if match else 0
    return name, ticks, wait, preempted, core


def core_sample():
    """Each core's (busy, total) ticks, from /proc/stat."""
    cores = {}
    for line in (read("/proc/stat") or "").splitlines():
        if line.startswith("cpu") and line[3:4].isdigit():
            name, *values = line.split()
            values = [int(value) for value in values]
            idle = values[3] + values[4]
            cores[int(name[3:])] = (sum(values) - idle, sum(values))
    return cores


def write_header(out):
    """What the machine is, and what the compositor renders with."""
    out.write(f"# probe {time.strftime('%Y-%m-%d %H:%M:%S')}\n")
    out.write(f"# kernel {platform.release()} {platform.machine()}\n")
    out.write(f"# cores {os.cpu_count()}\n")
    for core in range(os.cpu_count() or 0):
        base = f"/sys/devices/system/cpu/cpu{core}/cpufreq"
        governor = read(f"{base}/scaling_governor")
        if governor:
            out.write(
                f"# cpu{core} governor={governor} "
                f"min={read(f'{base}/scaling_min_freq')} "
                f"max={read(f'{base}/scaling_max_freq')}\n"
            )
    for name, paths in (("gpu_load", GPU_LOADS), ("gpu_freq", GPU_FREQS), ("emc", EMC_FREQS)):
        path, _ = first_readable(paths)
        out.write(f"# {name} {path or 'unavailable'}\n")
    for pid, name in watched_processes().items():
        if name != "labwc":
            continue
        environ = (read(f"/proc/{pid}/environ") or "").split("\0")
        hints = [item for item in environ if item.startswith(("WLR_", "LABWC_"))]
        out.write(f"# labwc pid={pid} env={' '.join(hints) or '-'}\n")
        libraries = sorted(set(RENDERER_LIBRARIES.findall(read(f"/proc/{pid}/maps") or "")))
        out.write(f"# labwc libraries={','.join(libraries) or '-'}\n")


class Probe:
    """The counters of the last sample, to write what changed since."""

    def __init__(self, out):
        self.out = out
        self.threads = {}
        self.cores = core_sample()
        self.sampled = now()

    def sample(self):
        """Write a sample."""
        elapsed = now() - self.sampled
        self.sampled = now()
        self.sample_threads(elapsed)
        self.sample_cores()
        self.sample_devices()
        self.out.flush()

    def sample_threads(self, elapsed):
        """Each watched thread's CPU use, wait and preemptions since the
        last sample."""
        threads = {}
        for pid, process in watched_processes().items():
            try:
                tids = [int(tid) for tid in os.listdir(f"/proc/{pid}/task")]
            except OSError:
                continue
            for tid in tids:
                sample = thread_sample(pid, tid)
                if sample is None:
                    continue
                threads[tid] = sample
                if tid in self.threads:
                    self.emit_thread((process, pid, tid), self.threads[tid], sample, elapsed)
        self.threads = threads

    def emit_thread(self, thread, last, sample, elapsed):
        """A thread's line: what changed from `last` to `sample`."""
        process, pid, tid = thread
        # name, CPU ticks, run-queue wait (ns), preemptions, core
        cpu = (sample[1] - last[1]) * 1000.0 / TICKS / elapsed * 100.0
        emit(
            self.out,
            "thread",
            proc=process,
            pid=pid,
            tid=tid,
            name=sample[0].replace(" ", "_"),
            cpu=f"{cpu:.1f}",
            wait=f"{(sample[2] - last[2]) / 1e6:.2f}",
            preempted=sample[3] - last[3],
            core=sample[4],
            ms=f"{elapsed:.1f}",
        )

    def sample_cores(self):
        """Each core's use and frequency."""
        cores = core_sample()
        for core, (busy, total) in cores.items():
            last_busy, last_total = self.cores.get(core, (busy, total))
            use = (busy - last_busy) * 100.0 / max(total - last_total, 1)
            freq = read(f"/sys/devices/system/cpu/cpu{core}/cpufreq/scaling_cur_freq")
            emit(self.out, "core", core=core, use=f"{use:.1f}", khz=freq or "-")
        self.cores = cores

    def sample_devices(self):
        """The GPU, the memory controller and the temperatures."""
        _, load = first_readable(GPU_LOADS)
        _, freq = first_readable(GPU_FREQS)
        _, emc = first_readable(EMC_FREQS)
        if load or freq or emc:
            emit(self.out, "gpu", load=load or "-", hz=freq or "-", emc=emc or "-")
        for zone in sorted(Path("/sys/class/thermal").glob("thermal_zone*")):
            temp = read(zone / "temp")
            if temp:
                kind = (read(zone / "type") or zone.name).replace(" ", "_")
                emit(self.out, "temp", zone=kind, millic=temp)


def main():
    """Sample until interrupted (or for --duration seconds)."""
    parser = argparse.ArgumentParser(description=__doc__.split("\n", maxsplit=1)[0])
    parser.add_argument("--out", default="/tmp/retrobox-probe.log", help="file to write")
    parser.add_argument("--interval", type=float, default=0.25, help="seconds between samples")
    parser.add_argument("--duration", type=float, default=0.0, help="seconds (0: until Ctrl+C)")
    args = parser.parse_args()
    with open(args.out, "w", encoding="utf-8") as out:
        write_header(out)
        probe = Probe(out)
        started = time.monotonic()
        print(f"probing into {args.out}; Ctrl+C to stop", file=sys.stderr)
        try:
            while not args.duration or time.monotonic() - started < args.duration:
                time.sleep(args.interval)
                probe.sample()
        except KeyboardInterrupt:
            pass
    print(f"done: {args.out}", file=sys.stderr)


if __name__ == "__main__":
    main()
