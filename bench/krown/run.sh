#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# Runs KROWN scenarios with KROWN's own execution framework, for RMLrs and
# Morph-KGC. See bench/krown/README.md.
#
#   bench/krown/run.sh <KROWN checkout> <scenario root> [runs]
#
# The scenario root must be under a directory Docker can bind-mount
# (on macOS: /Users, /private, /tmp, ...); it is mounted at the same path in
# the harness container, because the harness passes host paths to Docker.
#
# The patch() calls below quote short lines of KROWN's execution framework
# (MIT, see NOTICE) to find the code they change in the checkout.
set -eu

KROWN=$(cd "$1" && pwd)
ROOT=$(cd "$2" && pwd)
RUNS=${3:-3}
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
WORK=$(mktemp -d)
NET=rmlrs-bench_executor
WD=
trap 'test -n "$WD" && kill $WD 2>/dev/null; rm -rf "$WORK"; docker network rm $NET >/dev/null 2>&1 || true' EXIT

# Execution framework at the pinned commit, plus the RMLrs resource.
cp -R "$KROWN/execution-framework" "$WORK/ef"
cp "$HERE/rmlrs.py" "$WORK/ef/bench_executor/rmlrs.py"
# Local changes, none of which touch the measurements:
# - every container and the network get an "rmlrs-" prefix, so the run
#   cannot stop or replace containers it did not start;
# - Morph-KGC runs from an arm64 build of KROWN's own Dockerfile (the
#   published image is amd64 only), pinned to the version measured before.
python3 - "$WORK/ef/bench_executor" <<'PY'
import pathlib, sys
d = pathlib.Path(sys.argv[1])
def patch(name, old, new):
    p = d / name
    s = p.read_text()
    assert old in s, (name, old)
    p.write_text(s.replace(old, new, 1))
patch('container.py', "NETWORK_NAME = 'bench_executor'",
      "NETWORK_NAME = 'rmlrs-bench_executor'")
patch('docker.py', '        # Make sure the image is available locally',
      '        alias = name\n        name = f"rmlrs-{name}"\n'
      '        # Make sure the image is available locally')
# Engines reach the database by its usual name ("PostgreSQL") through a
# network alias; nothing is published on the host, so no host port is used.
patch('docker.py', "        cmd += f' --network \"{network}\"'",
      "        cmd += f' --network \"{network}\" --network-alias \"{alias}\"'")
patch('docker.py', 'for host_port, container_port in ports.items():',
      'for host_port, container_port in {}.items():')
# The harness itself joins the network and talks to PostgreSQL by name.
patch('postgresql.py', "HOST = 'localhost'", "HOST = 'PostgreSQL'")
# Database files live in the case directory (bind-mounted at the same
# path), not in /tmp of whatever machine runs the Docker daemon.
patch('postgresql.py',
      "tmp_dir = os.path.join(tempfile.gettempdir(), 'postgresql')",
      "tmp_dir = os.path.join(self._data_path, 'pgdata')")
patch('postgresql.py', "f'kgconstruct/postgresql:v{VERSION}'",
      "'rmlrs-krown/postgresql:14.5'")
# MySQL gets the same three changes as PostgreSQL. Its image is the official
# mysql:8.0 (Oracle Linux): KROWN's Dockerfile is mysql:8.0-debian, which is
# amd64 only; the server version is the same.
patch('mysql.py', "HOST = 'localhost'", "HOST = 'MySQL'")
patch('mysql.py',
      "tmp_dir = os.path.join(tempfile.gettempdir(), 'mysql')",
      "tmp_dir = os.path.join(self._data_path, 'mysqldata')")
patch('mysql.py', "f'kgconstruct/mysql:v{VERSION}'", "'mysql:8.0'")
# The server's config file is mounted from a path inside the harness
# container, which the Docker daemon cannot see; it is copied into the case
# directory instead, read-only for others (mysqld ignores a world-writable
# config file, and the resource sets umask 0).
patch('mysql.py',
      "        os.makedirs(os.path.join(self._data_path, 'mysql'), exist_ok=True)\n",
      "        os.makedirs(os.path.join(self._data_path, 'mysql'), exist_ok=True)\n"
      "        cnf = os.path.join(self._data_path, 'mysql',\n"
      "                           'mysql-secure-file-prive.cnf')\n"
      "        with open(os.path.join(self._config_path, 'mysql',\n"
      "                               'mysql-secure-file-prive.cnf')) as f:\n"
      "            text = f.read()\n"
      "        with open(cnf, 'w') as f:\n"
      "            f.write(text)\n"
      "        os.chmod(cnf, 0o644)\n")
patch('mysql.py', "f'{self._config_path}/mysql/'",
      "f'{self._data_path}/mysql/'")
patch('morphkgc.py', "f'kgconstruct/morph-kgc:v{VERSION}'",
      "'rmlrs-krown/morph-kgc:2.10.0'")
# Morph-KGC's process pool: without number_of_processes it starts one
# process per CPU (20 in a 10-CPU VM, which ran out of memory on 1M rows).
# RMLRS_MORPH_PROCESSES sets it; unset keeps the framework's default.
patch('morphkgc.py', '        # Morph-KGC can keep the mapping partition results separate, provide',
      "        if os.environ.get('RMLRS_MORPH_PROCESSES'):\n"
      "            config['CONFIGURATION']['number_of_processes'] = \\\n"
      "                os.environ['RMLRS_MORPH_PROCESSES']\n"
      '        # Morph-KGC can keep the mapping partition results separate, provide')
# psutil reports no CPU frequency inside Docker Desktop's arm64 VM; the
# collector only records it as hardware information.
patch('collector.py', 'cpu_min_freq = ps.cpu_freq().min',
      'cpu_min_freq = getattr(ps.cpu_freq(), "min", -1)')
patch('collector.py', 'cpu_max_freq = ps.cpu_freq().max',
      'cpu_max_freq = getattr(ps.cpu_freq(), "max", -1)')
PY

docker build -q -f "$HERE/Dockerfile" -t rmlrs-krown/rmlrs:dev "$REPO"
mkdir "$WORK/mk" "$WORK/h"
sed 's/pip3 install morph-kgc /pip3 install morph-kgc==2.10.0 /' \
    "$KROWN/execution-framework/dockers/Morph-KGC/Dockerfile" > "$WORK/mk/Dockerfile"
# Morph-KGC 2.x reads databases through SQLAlchemy, now an optional extra
# that KROWN's Dockerfile does not install (without it a database scenario
# ends without output). Added as a last layer, so the rest of the image
# stays the same.
echo 'RUN pip3 install "SQLAlchemy>=2.0.0"' >> "$WORK/mk/Dockerfile"
docker build -q -t rmlrs-krown/morph-kgc:2.10.0 "$WORK/mk"
cp "$KROWN/execution-framework/requirements.txt" "$WORK/h/"
docker build -q -f "$HERE/harness.Dockerfile" -t rmlrs-krown/harness "$WORK/h"
# KROWN's PostgreSQL image (kgconstruct/postgresql:v14.5) is amd64 only; its
# Dockerfile is just postgres:14.5-bullseye, built here for the host.
mkdir "$WORK/pg"
sed 's/${POSTGRESQL_VERSION}/14.5/' \
    "$KROWN/execution-framework/dockers/PostgreSQL/Dockerfile" > "$WORK/pg/Dockerfile"
docker build -q -t rmlrs-krown/postgresql:14.5 "$WORK/pg"

docker network inspect $NET >/dev/null 2>&1 || docker network create $NET >/dev/null

# Stops engine containers that were OOM-killed or run longer than
# RMLRS_STEP_TIMEOUT seconds (default 600); see watchdog.py.
python3 "$HERE/watchdog.py" "$ROOT/watchdog.log" "${RMLRS_STEP_TIMEOUT:-600}" &
WD=$!

docker run --rm --name rmlrs-krown-harness --network $NET -e RMLRS_MORPH_PROCESSES \
    -v /var/run/docker.sock:/var/run/docker.sock \
    -v "$WORK/ef:/ef" -v "$ROOT:$ROOT" -w /ef \
    rmlrs-krown/harness sh -c "./exectool run --root='$ROOT' --runs=$RUNS && ./exectool stats --root='$ROOT'"

