#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Summarise a KROWN run (after `exectool stats`) for RMLrs and Morph-KGC.

    bench/krown/summary.py <scenario root> [<scenario root> ...]

For every case: the median run that `exectool stats` picked, its duration,
CPU time (user + system) and RAM peak over the first sample of the step
(the collector records RAM of the whole machine), and the output of that
run: number of lines and an order-independent digest, so the outputs of
the two engines can be compared without sorting gigabytes of N-Triples.
Morph-KGC writes two spaces before the final dot; lines are normalised
before hashing.
"""

import csv
import hashlib
import os
import sys

ENGINES = {'RMLrs': 'rmlrs', 'MorphKGC': 'morphkgc'}


def output_digest(path):
    """(lines, digest) of an N-Triples file, independent of line order."""
    if not os.path.exists(path):
        if os.path.exists(path + '.digest'):  # left by compact.py
            lines, digest = open(path + '.digest').read().split()
            return int(lines), digest
        return None
    total, lines = 0, 0
    with open(path, 'rb') as f:
        for line in f:
            line = line.rstrip(b'\n')
            if not line:
                continue
            if line.endswith(b'  .'):
                line = line[:-3] + b' .'
            total += int.from_bytes(hashlib.md5(line).digest(), 'big')
            lines += 1
    return lines, f'{total % (1 << 128):032x}'[:12]


def median_run(results):
    runs = []
    for name in sorted(os.listdir(results)) if os.path.isdir(results) else []:
        path = os.path.join(results, name)
        if not (name.startswith('run_') and
                os.path.exists(os.path.join(path, '.done'))):
            continue
        rows = list(csv.DictReader(open(os.path.join(path, 'metrics.csv'))))
        step = max(r['step'] for r in rows)
        rows = [r for r in rows if r['step'] == step]
        runs.append({
            'run': name[4:], 'step': step,
            # Timestamps run on from the start of the run; a database
            # case's engine step starts after the load step.
            'duration': str(float(rows[-1]['timestamp'])
                            - float(rows[0]['timestamp'])),
            'cpu_user_system_diff': str(
                float(rows[-1]['cpu_user_system'])
                - float(rows[0]['cpu_user_system'])),
        })
    if not runs:
        return None
    runs.sort(key=lambda r: float(r['duration']))
    return runs[(len(runs) - 1) // 2]


def case_numbers(case_dir, engine):
    results = os.path.join(case_dir, 'results')
    summary = os.path.join(results, 'summary.csv')
    if os.path.exists(summary):
        rows = list(csv.DictReader(open(summary)))
        # Database scenarios have a loading step; the engine is the last.
        row = max(rows, key=lambda r: int(r['step']))
    else:
        # No `exectool stats` (the run was stopped): the median finished
        # run by duration, computed the same way from metrics.csv.
        row = median_run(results)
        if row is None:
            return None
    run, step = row['run'], row['step']
    metrics = [r for r in csv.DictReader(
        open(os.path.join(results, f'run_{run}', 'metrics.csv')))
        if r['step'] == step and int(r['memory_ram']) > 0]
    base = int(metrics[0]['memory_ram']) if metrics else 0
    peak = max((int(r['memory_ram']) for r in metrics), default=0)
    out = output_digest(os.path.join(results, f'run_{run}',
                                     ENGINES[engine], 'out.nt'))
    return {
        'duration': float(row['duration']),
        'cpu': float(row['cpu_user_system_diff']),
        'ram_mb': (peak - base) / 1e6,
        'out': out,
    }


def main(roots):
    print('| Case | Engine | Duration | CPU | RAM | Lines | Same output |')
    print('|---|---|---|---|---|---|---|')
    for root in roots:
        cases = {}
        for d, _, files in os.walk(root):
            if 'metadata.json' in files:
                rel = os.path.relpath(d, root).split(os.sep)
                if rel[0] in ENGINES:
                    cases.setdefault(rel[-1], {})[rel[0]] = d
        for case in sorted(cases):
            nums = {e: case_numbers(cases[case][e], e)
                    if e in cases[case] else None for e in ENGINES}
            outs = [n['out'] for n in nums.values() if n and n['out']]
            same = ('yes' if len(outs) == 2 and outs[0] == outs[1]
                    else 'NO' if len(outs) == 2 else '-')
            for engine, n in nums.items():
                if engine not in cases[case]:
                    continue
                if n is None:
                    print(f'| {case} | {engine} | did not finish | | | | |')
                    continue
                lines = f'{n["out"][0]:,}' if n['out'] else '-'
                print(f'| {case} | {engine} | {n["duration"]:.2f} s | '
                      f'{n["cpu"]:.1f} s | {n["ram_mb"]:,.0f} MB | {lines} | '
                      f'{same} |')


if __name__ == '__main__':
    main(sys.argv[1:])
