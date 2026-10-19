#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Stops engine containers that can no longer finish, so a run does not hang.

KROWN's framework waits for an engine container to exit, without a time
limit that fits a benchmark (6 hours). When the kernel kills one worker of
Morph-KGC's process pool for lack of memory, the pool waits for it forever:
the container keeps running at 0% CPU. This watchdog stops an engine
container (name `rmlrs-*`, except the harness and the database) when Docker
reports it OOM-killed, or when it has run longer than the timeout. The
framework then records the step as failed and goes on.

    watchdog.py <log file> [timeout seconds, default 600]
"""

import json
import subprocess
import sys
import time
from datetime import datetime, timezone

KEEP = {'rmlrs-krown-harness', 'rmlrs-PostgreSQL'}


def main(log_path, timeout):
    with open(log_path, 'a') as log:
        while True:
            names = subprocess.run(
                ['docker', 'ps', '--filter', 'name=rmlrs-',
                 '--format', '{{.Names}}'],
                capture_output=True, text=True).stdout.split()
            for name in names:
                if name in KEEP or not name.startswith('rmlrs-'):
                    continue
                out = subprocess.run(['docker', 'inspect', name],
                                     capture_output=True, text=True).stdout
                try:
                    state = json.loads(out)[0]['State']
                except (ValueError, IndexError, KeyError):
                    continue
                started = datetime.strptime(
                    state['StartedAt'][:19], '%Y-%m-%dT%H:%M:%S',
                ).replace(tzinfo=timezone.utc)
                age = (datetime.now(timezone.utc) - started).total_seconds()
                reason = ('OOMKilled' if state.get('OOMKilled')
                          else f'running {age:.0f}s' if age > timeout
                          else None)
                if reason:
                    stamp = datetime.now().isoformat(timespec='seconds')
                    log.write(f'{stamp} stop {name}: {reason}\n')
                    log.flush()
                    print(f'watchdog: stop {name}: {reason}', flush=True)
                    subprocess.run(['docker', 'stop', name],
                                   capture_output=True)
            time.sleep(5)


if __name__ == '__main__':
    main(sys.argv[1], int(sys.argv[2]) if len(sys.argv) > 2 else 600)
