// SPDX-License-Identifier: Apache-2.0
//! Parsers for RMLrs: Turtle (mapping documents), N-Quads (test outputs),
//! string templates, and the RML vocabulary on top of Turtle.

mod legacy;
pub mod nquads;
pub mod rml;
pub mod template;
pub mod turtle;

pub use legacy::PROCESSOR_DATABASE;

use rmlrs_model::{Mapping, Result};

/// Parses an RML mapping document written in Turtle. Mappings in the
/// legacy R2RML and RML vocabularies are read too.
pub fn parse_mapping(turtle_text: &str, base: Option<&str>) -> Result<Mapping> {
    rml::mapping_from_triples(turtle::parse(turtle_text, base)?)
}
