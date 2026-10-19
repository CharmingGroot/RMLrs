#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Replaces the engine outputs of finished runs by their digest.

The framework keeps every run's output; at 1M members that is 1.3 GB a
run. For each finished run (`.done`), this writes `out.nt.digest` (lines
and the order-independent digest that summary.py compares) and deletes
`out.nt`.

    compact.py <scenario root>
"""

import os
import sys

from summary import output_digest


def main(root):
    for d, _, files in os.walk(root):
        if 'out.nt' in files and os.path.exists(
                os.path.join(os.path.dirname(d), '.done')):
            path = os.path.join(d, 'out.nt')
            lines, digest = output_digest(path)
            with open(path + '.digest', 'w') as f:
                f.write(f'{lines} {digest}\n')
            os.remove(path)


if __name__ == '__main__':
    main(sys.argv[1])
