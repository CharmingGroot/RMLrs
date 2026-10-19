// SPDX-License-Identifier: Apache-2.0
//! RMLrs: a Rust engine for RML, the RDF Mapping Language.
//!
//! ```text
//! mapping + source data  ->  RDF quads
//! ```
//!
//! ```no_run
//! let quads = rmlrs::run_file("mapping.ttl", "http://example.com/")?;
//! print!("{}", rmlrs::to_nquads(&quads));
//! # Ok::<(), rmlrs::Error>(())
//! ```

use std::path::Path;

pub use rmlrs_engine::{Destination, Options, run, run_to, run_to_destinations};
pub use rmlrs_model::{
    DatabaseSource, Error, Literal, Mapping, Quad, RemoteSource, Result, Term, Triple,
};
pub use rmlrs_parser::{nquads, parse_mapping, turtle};
pub use rmlrs_writer::{TargetWriter, to_nquads, write_nquads};

use rmlrs_model::PathRoot;

/// Reads a mapping document and runs it. Relative source paths resolve
/// against the mapping's directory or the current working directory, as the
/// mapping says.
pub fn run_file(mapping_path: impl AsRef<Path>, base_iri: &str) -> Result<Vec<Quad>> {
    let mut out = Vec::new();
    run_file_to(mapping_path, base_iri, &mut |q| {
        out.push(q);
        Ok(())
    })?;
    Ok(out)
}

/// Like [`run_file`], but hands each quad to `sink` as soon as it exists.
pub fn run_file_to(
    mapping_path: impl AsRef<Path>,
    base_iri: &str,
    sink: &mut dyn FnMut(Quad) -> Result<()>,
) -> Result<()> {
    let path = mapping_path.as_ref();
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Source(format!("{}: {e}", path.display())))?;
    let mapping = parse_mapping(&text, None)?;
    let mapping_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let working_dir = std::env::current_dir().map_err(|e| Error::Source(e.to_string()))?;
    run_with_targets(
        &mapping,
        &Options {
            base_iri: base_iri.to_owned(),
            mapping_dir,
            working_dir,
            ..Default::default()
        },
        sink,
    )
}

/// Runs a mapping, writes the quads of its logical targets to their files,
/// and hands the default output to `default_sink`. Every target file is
/// created, even when no quad goes there.
pub fn run_with_targets(
    mapping: &Mapping,
    options: &Options,
    default_sink: &mut dyn FnMut(Quad) -> Result<()>,
) -> Result<()> {
    let mut writers = mapping
        .logical_targets
        .iter()
        .map(|t| {
            let dir = match t.root {
                PathRoot::MappingDirectory => &options.mapping_dir,
                PathRoot::CurrentWorkingDirectory => &options.working_dir,
            };
            TargetWriter::create(
                &dir.join(&t.path),
                t.serialization,
                t.encoding,
                t.compression,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    run_to_destinations(mapping, options, &mut |q, d| match d {
        Destination::Default => default_sink(q),
        Destination::Target(i) => writers[i].write(&q),
    })?;
    for w in writers {
        w.finish()?;
    }
    Ok(())
}

/// Runs a mapping given as Turtle text over documents held in memory, keyed
/// by the `rml:path` the mapping uses. Paths not in `documents` are read from
/// `mapping_dir`. This is the entry point the Python and WebAssembly bindings
/// use.
pub fn run_in_memory(
    mapping_turtle: &str,
    base_iri: &str,
    documents: std::collections::HashMap<String, String>,
    mapping_dir: impl Into<std::path::PathBuf>,
) -> Result<Vec<Quad>> {
    let mapping = parse_mapping(mapping_turtle, None)?;
    let dir = mapping_dir.into();
    run(
        &mapping,
        &Options {
            base_iri: base_iri.to_owned(),
            mapping_dir: dir.clone(),
            working_dir: dir,
            in_memory: documents,
            ..Default::default()
        },
    )
}
