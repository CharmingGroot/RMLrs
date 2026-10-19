# SPDX-License-Identifier: Apache-2.0
"""Runs every RML-Core case through the Python module and compares the output
with the native CLI, byte for byte. Usage:

    PYTHONPATH=<dir with rmlrs.abi3.so> python3 core_parity.py <rmlrs binary> <suites dir>
"""
import csv
import pathlib
import subprocess
import sys

import rmlrs

binary, suites = str(pathlib.Path(sys.argv[1]).resolve()), pathlib.Path(sys.argv[2]).resolve()
cases = suites / "rml-core" / "test-cases"
same = differ = 0
for row in csv.DictReader(open(cases / "metadata.csv")):
    d = cases / row["ID"]
    mapping = (d / row["mapping"]).read_text()
    base = row["base_iri"] or "http://example.com/"
    native = subprocess.run([binary, "run", str(d / row["mapping"]), "--base-iri", base], capture_output=True, text=True, cwd=d)
    try:
        py = rmlrs.materialize(mapping, base_iri=base, mapping_dir=str(d))
        py_ok = True
    except Exception:  # noqa: BLE001
        py, py_ok = "", False
    if (native.returncode == 0) == py_ok and (not py_ok or native.stdout == py):
        same += 1
    else:
        differ += 1
        print("differs:", row["ID"])
print(f"same {same} / {same + differ}")
sys.exit(1 if differ else 0)
