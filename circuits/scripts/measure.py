#!/usr/bin/env python3
"""Run a command and report wall time and peak resident memory of it and its children.

Usage: measure.py <command> [args...]   (wrap in systemd-run for a hard memory cap)
"""
import resource
import subprocess
import sys
import time

start = time.time()
code = subprocess.call(sys.argv[1:])
wall = time.time() - start
peak_kib = resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss
print(f"[measure] exit={code} wall={wall:.1f}s peak_rss={peak_kib / 1024:.0f}MiB", file=sys.stderr)
sys.exit(code)
