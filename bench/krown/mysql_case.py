#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""
Turns a KROWN case generated with `data_format: postgresql` into the same
case on MySQL. See bench/krown/README.md.

    bench/krown/mysql_case.py <postgresql case directory> <new case directory>

KROWN's execution framework has a MySQL resource, but its data generator
(at 824438c) writes database steps only for PostgreSQL: `mysql` is listed as
a data format and then raises NotImplementedError. The data and the mapping
of a PostgreSQL case are what a MySQL case would hold too (the mapping only
names the table), so this copies them and changes the steps' database:
resource, host, port and type. Nothing else in the case changes.
"""

import json
import os
import shutil
import sys

SHARED = os.path.join('data', 'shared')


def main(src: str, dst: str) -> None:
    with open(os.path.join(src, 'metadata.json')) as f:
        metadata = json.load(f)
    for step in metadata['steps']:
        if step['resource'] == 'PostgreSQL':
            step['resource'] = 'MySQL'
        params = step['parameters']
        if params.get('rdb_type') == 'PostgreSQL':
            params.update(rdb_host='MySQL', rdb_port=3306, rdb_type='MySQL')
    os.makedirs(os.path.join(dst, SHARED))
    for name in sorted(os.listdir(os.path.join(src, SHARED))):
        # Only the generator's files: data and the original mapping.
        if name.endswith('.csv') and not name.startswith('tmp_') \
                or name.endswith('.r2rml.ttl'):
            shutil.copy(os.path.join(src, SHARED, name),
                        os.path.join(dst, SHARED, name))
    with open(os.path.join(dst, 'metadata.json'), 'w') as f:
        json.dump(metadata, f, indent=4)


if __name__ == '__main__':
    main(sys.argv[1], sys.argv[2])
