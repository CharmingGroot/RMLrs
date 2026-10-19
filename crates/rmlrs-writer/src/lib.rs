// SPDX-License-Identifier: Apache-2.0
//! Output: N-Quads for the default output, and the serialisations and
//! encodings of RML-IO logical targets (N-Quads, N-Triples, Turtle, N3,
//! RDF/JSON, JSON-LD and RDF/XML).

mod compression;
mod jsonld;
mod rdfxml;

use std::collections::BTreeMap;
use std::io::{self, BufWriter, Write};
use std::path::Path;

use rmlrs_model::{
    Compression, Error, Quad, Result, Serialization, Term, TextEncoding, XSD_STRING,
};

use compression::Sink;

/// Writes quads as N-Quads, one per line.
pub fn write_nquads<'a, W: Write>(
    out: &mut W,
    quads: impl IntoIterator<Item = &'a Quad>,
) -> io::Result<()> {
    for q in quads {
        writeln!(out, "{q}")?;
    }
    Ok(())
}

/// N-Quads as a string.
pub fn to_nquads<'a>(quads: impl IntoIterator<Item = &'a Quad>) -> String {
    let mut buf = Vec::new();
    write_nquads(&mut buf, quads).expect("writing to a Vec does not fail");
    String::from_utf8(buf).expect("quads are UTF-8")
}

/// Writes the quads of one logical target to a file.
#[derive(Debug)]
pub struct TargetWriter {
    serialization: Serialization,
    encoding: TextEncoding,
    out: BufWriter<Sink>,
    /// RDF/JSON needs all triples grouped by subject before writing.
    rdf_json: BTreeMap<String, BTreeMap<String, Vec<serde_json::Value>>>,
    /// JSON-LD and RDF/XML group quads by graph and subject, so they are
    /// written when the target is finished.
    held: Vec<Quad>,
    line: String,
}

impl TargetWriter {
    pub fn create(
        path: &Path,
        serialization: Serialization,
        encoding: TextEncoding,
        compression: Compression,
    ) -> Result<Self> {
        let mut out = BufWriter::with_capacity(1 << 16, Sink::create(path, compression)?);
        if encoding == TextEncoding::Utf16 {
            out.write_all(&[0xFF, 0xFE])
                .map_err(|e| target_error(path, e))?; // UTF-16LE byte order mark
        }
        Ok(TargetWriter {
            serialization,
            encoding,
            out,
            rdf_json: BTreeMap::new(),
            held: Vec::new(),
            line: String::new(),
        })
    }

    pub fn write(&mut self, q: &Quad) -> Result<()> {
        use std::fmt::Write as _;
        // The parser refuses mappings that send named graphs to such targets
        // (RML-IO, section 7.4); this catches the triples it cannot foresee.
        if let Some(g) = &q.graph
            && !self.serialization.has_named_graphs()
        {
            return Err(Error::Mapping(format!(
                "{:?} targets cannot hold named graphs, got a triple in graph {g}",
                self.serialization
            )));
        }
        self.line.clear();
        match self.serialization {
            Serialization::NQuads => {
                let _ = writeln!(self.line, "{q}");
            }
            // The triple only: named graphs were refused above. N-Triples
            // lines are also valid Turtle and N3.
            Serialization::NTriples | Serialization::Turtle | Serialization::N3 => {
                let _ = writeln!(self.line, "{} {} {} .", q.subject, q.predicate, q.object);
            }
            Serialization::RdfJson => {
                let subject = match &q.subject {
                    Term::Iri(i) => i.clone(),
                    Term::BlankNode(b) => format!("_:{b}"),
                    Term::Literal(_) => return Err(Error::Data("literal subject".into())),
                    Term::Triple(_) => {
                        return Err(Error::Unsupported("quoted triples in RDF/JSON".into()));
                    }
                };
                if let Term::Triple(_) = q.object {
                    return Err(Error::Unsupported("quoted triples in RDF/JSON".into()));
                }
                let predicate = q.predicate.as_iri().unwrap_or_default().to_owned();
                self.rdf_json
                    .entry(subject)
                    .or_default()
                    .entry(predicate)
                    .or_default()
                    .push(rdf_json_object(&q.object));
                return Ok(());
            }
            Serialization::JsonLd => {
                jsonld::check(q)?;
                self.held.push(q.clone());
                return Ok(());
            }
            Serialization::RdfXml => {
                rdfxml::check(q)?;
                self.held.push(q.clone());
                return Ok(());
            }
        }
        let line = std::mem::take(&mut self.line);
        let r = self.put(&line);
        self.line = line;
        r
    }

    fn put(&mut self, text: &str) -> Result<()> {
        let r = match self.encoding {
            TextEncoding::Utf8 => self.out.write_all(text.as_bytes()),
            TextEncoding::Utf16 => {
                let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
                self.out.write_all(&bytes)
            }
        };
        r.map_err(|e| Error::Source(format!("writing target: {e}")))
    }

    pub fn finish(mut self) -> Result<()> {
        let document = match self.serialization {
            Serialization::RdfJson => {
                Some(serde_json::to_string_pretty(&self.rdf_json).expect("JSON values serialise"))
            }
            Serialization::JsonLd => Some(jsonld::serialize(&self.held)?),
            Serialization::RdfXml => Some(rdfxml::serialize(
                &self.held,
                match self.encoding {
                    TextEncoding::Utf8 => "UTF-8",
                    TextEncoding::Utf16 => "UTF-16",
                },
            )?),
            _ => None,
        };
        if let Some(document) = document {
            self.put(&document)?;
        }
        let sink = self
            .out
            .into_inner()
            .map_err(|e| Error::Source(format!("writing target: {}", e.error())))?;
        sink.finish()
            .map_err(|e| Error::Source(format!("writing target: {e}")))
    }
}

fn target_error(path: &Path, e: io::Error) -> Error {
    Error::Source(format!("target {}: {e}", path.display()))
}

/// The RDF/JSON form of an object (W3C Note "RDF 1.1 JSON Alternate Serialization").
fn rdf_json_object(t: &Term) -> serde_json::Value {
    use serde_json::json;
    match t {
        Term::Iri(i) => json!({ "type": "uri", "value": i }),
        Term::BlankNode(b) => json!({ "type": "bnode", "value": format!("_:{b}") }),
        Term::Literal(l) => {
            let mut v = json!({ "type": "literal", "value": l.value });
            if let Some(lang) = &l.language {
                v["lang"] = json!(lang);
            } else if l.datatype != XSD_STRING {
                v["datatype"] = json!(l.datatype);
            }
            v
        }
        Term::Triple(_) => unreachable!("rejected by Writer::write"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmlrs_model::{Literal, Term};

    #[test]
    fn one_line_per_quad() {
        let q = Quad {
            subject: Term::iri("http://e/s"),
            predicate: Term::iri("http://e/p"),
            object: Term::Literal(Literal::simple("o")),
            graph: None,
        };
        assert_eq!(to_nquads([&q]), "<http://e/s> <http://e/p> \"o\" .\n");
    }

    #[test]
    fn named_graphs_only_go_to_formats_that_hold_them() {
        let dir = std::env::temp_dir().join(format!("rmlrs-writer-graphs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut q = Quad {
            subject: Term::iri("http://e/s"),
            predicate: Term::iri("http://e/p"),
            object: Term::iri("http://e/o"),
            graph: Some(Term::iri("http://e/g")),
        };
        let open = |s| {
            TargetWriter::create(&dir.join("out"), s, TextEncoding::Utf8, Compression::None)
                .unwrap()
        };
        use Serialization::*;
        for s in [NTriples, Turtle, N3, RdfJson, RdfXml] {
            let mut w = open(s);
            let e = w.write(&q).unwrap_err();
            assert!(matches!(e, Error::Mapping(_)), "{s:?}: {e}");
            assert!(e.to_string().contains("named graphs"), "{s:?}: {e}");
        }
        for s in [NQuads, JsonLd] {
            let mut w = open(s);
            w.write(&q).unwrap();
            w.finish().unwrap();
        }
        // The default graph is fine everywhere.
        q.graph = None;
        for s in [NTriples, Turtle, N3, RdfJson, RdfXml, NQuads, JsonLd] {
            let mut w = open(s);
            w.write(&q).unwrap();
            w.finish().unwrap();
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
