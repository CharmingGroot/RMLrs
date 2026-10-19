// SPDX-License-Identifier: Apache-2.0
//! Python bindings: a thin wrapper over `rmlrs::run_in_memory`.
//!
//! ```python
//! import rmlrs
//! nquads = rmlrs.materialize(mapping_ttl, sources={"data.json": text})
//! ```

use std::collections::HashMap;

use pyo3::exceptions::{PyNotImplementedError, PyOSError, PyValueError};
use pyo3::prelude::*;

fn to_py(e: rmlrs::Error) -> PyErr {
    match e {
        rmlrs::Error::Unsupported(_) => PyNotImplementedError::new_err(e.to_string()),
        rmlrs::Error::Source(_) => PyOSError::new_err(e.to_string()),
        _ => PyValueError::new_err(e.to_string()),
    }
}

/// Runs an RML mapping (Turtle text) and returns the output as N-Quads.
///
/// `sources` maps the `rml:path` values of the mapping to documents held in
/// memory; other paths are read relative to `mapping_dir`.
#[pyfunction]
#[pyo3(signature = (mapping, base_iri = "http://example.com/", sources = None, mapping_dir = "."))]
fn materialize(
    py: Python<'_>,
    mapping: &str,
    base_iri: &str,
    sources: Option<HashMap<String, String>>,
    mapping_dir: &str,
) -> PyResult<String> {
    let documents = sources.unwrap_or_default();
    let quads = py
        .detach(|| rmlrs::run_in_memory(mapping, base_iri, documents, mapping_dir))
        .map_err(to_py)?;
    Ok(rmlrs::to_nquads(&quads))
}

#[pymodule(name = "rmlrs")]
fn rmlrs_module(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(materialize, m)?)?;
    m.add("__version__", env!("CARGO_PKG_VERSION"))?;
    Ok(())
}
