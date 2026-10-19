# rmlrs (Python)

A thin Python wrapper over the RMLrs library. The output is the same as the
native CLI, byte for byte, on all 76 RML-Core cases (`tests/core_parity.py`).

```python
import rmlrs

nquads = rmlrs.materialize(
    mapping_ttl,                        # the RML mapping, as Turtle text
    base_iri="http://example.com/",
    sources={"data.json": json_text},   # documents in memory, keyed by rml:path
    mapping_dir=".",                    # other paths are read from here
)
```

Errors: `ValueError` for invalid mappings and data, `OSError` for unreadable
sources, `NotImplementedError` for RML features RMLrs does not implement yet.

## Build

With [maturin](https://www.maturin.rs/) (one abi3 wheel for Python 3.9+):

```bash
cd bindings/python
maturin build --release
pip install ../../target/wheels/rmlrs-*.whl
```

The output is N-Quads text, so any RDF store or library that reads N-Quads
can load it directly.

Without maturin, for a local check on macOS or Linux:

```bash
cargo build --release -p rmlrs-python
cp target/release/librmlrs.dylib /tmp/rmlrs.abi3.so   # .so on Linux
PYTHONPATH=/tmp python3 bindings/python/tests/core_parity.py target/release/rmlrs conformance/suites
```
