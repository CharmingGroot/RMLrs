// SPDX-License-Identifier: Apache-2.0
//! RDF terms, quads, errors and the RML mapping model shared by the RMLrs crates.

pub mod iri;
pub mod mapping;
pub mod rdf;

pub use mapping::*;
pub use rdf::*;

/// What went wrong, sorted by who has to fix it.
///
/// The conformance runner relies on this split: a test case that expects an
/// error passes only on [`Error::Mapping`] or [`Error::Data`], never on
/// [`Error::Unsupported`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The mapping document is not a valid RML mapping.
    #[error("invalid mapping: {0}")]
    Mapping(String),
    /// The mapping is valid, but the data makes it produce something invalid
    /// (for example a template value that does not form an IRI).
    #[error("data error: {0}")]
    Data(String),
    /// A source could not be read.
    #[error("source error: {0}")]
    Source(String),
    /// The mapping uses something RMLrs does not implement yet.
    #[error("not supported: {0}")]
    Unsupported(String),
    /// Turtle or N-Quads syntax error.
    #[error("syntax error: {0}")]
    Syntax(String),
}

impl Error {
    /// True when the error reflects a real problem in the mapping or data,
    /// as opposed to a missing feature.
    pub fn is_spec_error(&self) -> bool {
        !matches!(self, Error::Unsupported(_))
    }
}

pub type Result<T> = std::result::Result<T, Error>;
