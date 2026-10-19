// SPDX-License-Identifier: Apache-2.0
//! WebAssembly bindings: a thin wrapper over `rmlrs::run_in_memory`.
//!
//! There is no file system in the browser, so every source the mapping
//! reads must be passed in `sources`, keyed by its `rml:path`.
//!
//! ```js
//! import { materialize } from "rmlrs";
//! const nquads = materialize(mappingTtl, { "data.json": text });
//! ```

use std::collections::HashMap;

use wasm_bindgen::prelude::*;

/// Runs an RML mapping (Turtle text) over in-memory documents and returns
/// N-Quads. `sources` is a plain object from `rml:path` to document text.
#[wasm_bindgen]
pub fn materialize(
    mapping: &str,
    sources: &js_sys::Object,
    base_iri: Option<String>,
) -> Result<String, JsError> {
    let mut documents = HashMap::new();
    for entry in js_sys::Object::entries(sources).iter() {
        let pair = js_sys::Array::from(&entry);
        let (Some(key), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string()) else {
            return Err(JsError::new("sources must map paths to strings"));
        };
        documents.insert(key, value);
    }
    let base = base_iri.unwrap_or_else(|| "http://example.com/".to_owned());
    let quads = rmlrs::run_in_memory(mapping, &base, documents, "")
        .map_err(|e| JsError::new(&e.to_string()))?;
    Ok(rmlrs::to_nquads(&quads))
}

/// The RMLrs version.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}
