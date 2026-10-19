// SPDX-License-Identifier: Apache-2.0
//! RDF/XML (W3C RDF 1.1 XML Syntax) for logical targets.
//!
//! Triples are grouped by subject into `rdf:Description` elements. Subjects
//! and IRI objects are written in full (`rdf:about`, `rdf:resource`), blank
//! nodes with `rdf:nodeID`, literals as element text with `xml:lang` or
//! `rdf:datatype`. Only predicates need a qualified name: the IRI is split
//! into a namespace and the longest local part that is an XML NCName, and
//! an IRI with no such local part cannot be written.
//!
//! RDF/XML has no named graphs. RML-IO (section 7.4) calls a mapping that
//! sends a named graph to such a target invalid, so a quad in a named graph
//! is a mapping error. Quoted triples (RDF-star) are not supported.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

use rmlrs_model::{Error, Quad, RDF, Result, Term, XSD_STRING};

const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS_NS: &str = "http://www.w3.org/2000/xmlns/";

/// RDF/XML syntax names that cannot be property elements (RDF 1.1 XML
/// Syntax, 7.2.5: coreSyntaxTerms, oldTerms, rdf:Description, and rdf:li,
/// which a reader turns into rdf:_n).
const NOT_PROPERTIES: [&str; 12] = [
    "RDF",
    "ID",
    "about",
    "parseType",
    "resource",
    "nodeID",
    "datatype",
    "Description",
    "li",
    "aboutEach",
    "aboutEachPrefix",
    "bagID",
];

/// Rejects, as early as possible, a quad RDF/XML cannot hold.
pub(crate) fn check(q: &Quad) -> Result<()> {
    if let Some(g) = &q.graph {
        return Err(Error::Mapping(format!(
            "RDF/XML targets cannot hold named graphs, got a triple in graph {g}"
        )));
    }
    for t in [&q.subject, &q.object] {
        if let Term::Triple(_) = t {
            return Err(Error::Unsupported("quoted triples in RDF/XML".into()));
        }
    }
    if let Term::Literal(_) = q.subject {
        return Err(Error::Data("literal subject".into()));
    }
    let p = q.predicate.as_iri().unwrap_or_default();
    split(p)?;
    Ok(())
}

/// Splits a predicate IRI into namespace and local name.
fn split(iri: &str) -> Result<(&str, &str)> {
    let suffix = iri
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_name_char(c))
        .last()
        .map_or(iri.len(), |(i, _)| i);
    let start = iri[suffix..]
        .char_indices()
        .find(|&(_, c)| is_name_start_char(c))
        .map_or(iri.len(), |(i, _)| suffix + i);
    let (ns, local) = iri.split_at(start);
    if local.is_empty() || ns.is_empty() {
        return Err(Error::Data(format!(
            "predicate <{iri}> cannot be written in RDF/XML: it does not end in an XML name"
        )));
    }
    if ns == RDF && NOT_PROPERTIES.contains(&local) {
        return Err(Error::Data(format!(
            "predicate <{iri}> cannot be written in RDF/XML: it is a syntax name"
        )));
    }
    if ns == XMLNS_NS {
        return Err(Error::Data(format!(
            "predicate <{iri}> cannot be written in RDF/XML: reserved namespace"
        )));
    }
    Ok((ns, local))
}

/// XML 1.0 NameStartChar, without ':'.
fn is_name_start_char(c: char) -> bool {
    matches!(c,
        'A'..='Z' | '_' | 'a'..='z'
        | '\u{C0}'..='\u{D6}' | '\u{D8}'..='\u{F6}' | '\u{F8}'..='\u{2FF}'
        | '\u{370}'..='\u{37D}' | '\u{37F}'..='\u{1FFF}' | '\u{200C}'..='\u{200D}'
        | '\u{2070}'..='\u{218F}' | '\u{2C00}'..='\u{2FEF}' | '\u{3001}'..='\u{D7FF}'
        | '\u{F900}'..='\u{FDCF}' | '\u{FDF0}'..='\u{FFFD}' | '\u{10000}'..='\u{EFFFF}')
}

/// XML 1.0 NameChar, without ':'.
fn is_name_char(c: char) -> bool {
    is_name_start_char(c)
        || matches!(c,
            '-' | '.' | '0'..='9' | '\u{B7}' | '\u{300}'..='\u{36F}' | '\u{203F}'..='\u{2040}')
}

fn is_ncname(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(is_name_start_char) && chars.all(is_name_char)
}

/// Characters XML 1.0 can hold at all, escaped or not.
fn is_xml_char(c: char) -> bool {
    matches!(c,
        '\t' | '\n' | '\r' | '\u{20}'..='\u{D7FF}' | '\u{E000}'..='\u{FFFD}' | '\u{10000}'..)
}

/// Escapes text content (`attr == false`) or a double-quoted attribute value.
/// Carriage returns, and tabs and newlines in attributes, are written as
/// character references: XML parsers would normalise them otherwise.
fn escape(s: &str, attr: bool, out: &mut String) -> Result<()> {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attr => out.push_str("&quot;"),
            '\t' if attr => out.push_str("&#x9;"),
            '\n' if attr => out.push_str("&#xA;"),
            '\r' => out.push_str("&#xD;"),
            c if !is_xml_char(c) => {
                return Err(Error::Data(format!(
                    "U+{:04X} cannot be written in XML",
                    u32::from(c)
                )));
            }
            c => out.push(c),
        }
    }
    Ok(())
}

/// Writes the whole document. `encoding` goes into the XML declaration.
pub(crate) fn serialize(quads: &[Quad], encoding: &str) -> Result<String> {
    let mut subjects: BTreeMap<&Term, Vec<&Quad>> = BTreeMap::new();
    let mut labels = BTreeSet::new();
    for q in quads {
        check(q)?;
        subjects.entry(&q.subject).or_default().push(q);
        for t in [&q.subject, &q.object] {
            if let Term::BlankNode(b) = t {
                labels.insert(b.as_str());
            }
        }
    }
    // rdf:nodeID must be an NCName; other labels get fresh ones.
    let mut node_ids: HashMap<&str, String> = HashMap::new();
    let mut fresh = 0usize;
    for &b in &labels {
        let id = if is_ncname(b) {
            b.to_owned()
        } else {
            loop {
                fresh += 1;
                let id = format!("b{fresh}");
                if !labels.contains(id.as_str()) {
                    break id;
                }
            }
        };
        node_ids.insert(b, id);
    }

    let mut prefixes: BTreeMap<&str, String> = BTreeMap::new();
    prefixes.insert(RDF, "rdf".into());
    prefixes.insert(XML_NS, "xml".into());
    let mut body = String::new();
    for (subject, triples) in subjects {
        body.push_str("  <rdf:Description ");
        node_attr(subject, "about", &node_ids, &mut body)?;
        body.push_str(">\n");
        for q in triples {
            let (ns, local) = split(q.predicate.as_iri().unwrap_or_default())?;
            let n = prefixes.len() - 1;
            let prefix = prefixes.entry(ns).or_insert_with(|| format!("ns{n}"));
            let name = format!("{prefix}:{local}");
            let _ = write!(body, "    <{name}");
            match &q.object {
                Term::Literal(l) => {
                    if let Some(lang) = &l.language {
                        body.push_str(" xml:lang=\"");
                        escape(lang, true, &mut body)?;
                        body.push('"');
                    } else if l.datatype != XSD_STRING {
                        body.push_str(" rdf:datatype=\"");
                        escape(&l.datatype, true, &mut body)?;
                        body.push('"');
                    }
                    body.push('>');
                    escape(&l.value, false, &mut body)?;
                    let _ = writeln!(body, "</{name}>");
                }
                other => {
                    body.push(' ');
                    node_attr(other, "resource", &node_ids, &mut body)?;
                    body.push_str("/>\n");
                }
            }
        }
        body.push_str("  </rdf:Description>\n");
    }

    let mut out = format!("<?xml version=\"1.0\" encoding=\"{encoding}\"?>\n<rdf:RDF");
    for (ns, prefix) in &prefixes {
        if *ns == XML_NS {
            continue; // bound by XML itself
        }
        let _ = write!(out, "\n    xmlns:{prefix}=\"");
        escape(ns, true, &mut out)?;
        out.push('"');
    }
    out.push_str(">\n");
    out.push_str(&body);
    out.push_str("</rdf:RDF>\n");
    Ok(out)
}

/// `rdf:about` / `rdf:resource` for an IRI, `rdf:nodeID` for a blank node.
fn node_attr(
    t: &Term,
    iri_attr: &str,
    node_ids: &HashMap<&str, String>,
    out: &mut String,
) -> Result<()> {
    match t {
        Term::Iri(i) => {
            let _ = write!(out, "rdf:{iri_attr}=\"");
            escape(i, true, out)?;
        }
        Term::BlankNode(b) => {
            out.push_str("rdf:nodeID=\"");
            out.push_str(&node_ids[b.as_str()]);
        }
        _ => unreachable!("rejected by check"),
    }
    out.push('"');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmlrs_model::Literal;

    fn quad(s: Term, p: &str, o: Term) -> Quad {
        Quad {
            subject: s,
            predicate: Term::iri(p),
            object: o,
            graph: None,
        }
    }

    fn xml(quads: &[Quad]) -> String {
        serialize(quads, "UTF-8").unwrap()
    }

    #[test]
    fn groups_by_subject_with_declared_namespaces() {
        let out = xml(&[
            quad(
                Term::iri("http://e/s"),
                "http://xmlns.com/foaf/0.1/name",
                Term::Literal(Literal::simple("Monica")),
            ),
            quad(
                Term::iri("http://e/s"),
                "http://www.w3.org/1999/02/22-rdf-syntax-ns#type",
                Term::iri("http://e/Person"),
            ),
        ]);
        assert_eq!(
            out,
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
             <rdf:RDF\n    xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"\n    \
             xmlns:ns1=\"http://xmlns.com/foaf/0.1/\">\n  \
             <rdf:Description rdf:about=\"http://e/s\">\n    \
             <ns1:name>Monica</ns1:name>\n    \
             <rdf:type rdf:resource=\"http://e/Person\"/>\n  \
             </rdf:Description>\n</rdf:RDF>\n"
        );
    }

    #[test]
    fn escapes_text_and_attributes() {
        let out = xml(&[quad(
            Term::iri("http://e/s?a=1&b=\"2\""),
            "http://e/p",
            Term::Literal(Literal::simple("a & b < c > d \"q\"\r\n")),
        )]);
        assert!(out.contains("rdf:about=\"http://e/s?a=1&amp;b=&quot;2&quot;\""));
        assert!(out.contains(">a &amp; b &lt; c &gt; d \"q\"&#xD;\n</ns1:p>"));
    }

    #[test]
    fn newlines_and_tabs_in_attributes_are_references() {
        let mut out = String::new();
        escape("a\tb\nc", true, &mut out).unwrap();
        assert_eq!(out, "a&#x9;b&#xA;c");
        out.clear();
        escape("a\tb\nc", false, &mut out).unwrap();
        assert_eq!(out, "a\tb\nc");
    }

    #[test]
    fn language_and_datatype() {
        let out = xml(&[
            quad(
                Term::iri("http://e/s"),
                "http://e/p",
                Term::Literal(Literal::lang("chat", "fr")),
            ),
            quad(
                Term::iri("http://e/s"),
                "http://e/q",
                Term::Literal(Literal::typed(
                    "33",
                    "http://www.w3.org/2001/XMLSchema#integer",
                )),
            ),
        ]);
        assert!(out.contains("<ns1:p xml:lang=\"fr\">chat</ns1:p>"));
        assert!(out.contains(
            "<ns1:q rdf:datatype=\"http://www.w3.org/2001/XMLSchema#integer\">33</ns1:q>"
        ));
        assert!(!out.contains("xmlns:xml"));
    }

    #[test]
    fn blank_nodes_use_node_ids() {
        let out = xml(&[
            quad(
                Term::BlankNode("x1".into()),
                "http://e/p",
                Term::BlankNode("not an NCName".into()),
            ),
            quad(
                Term::BlankNode("not an NCName".into()),
                "http://e/p",
                Term::BlankNode("b1".into()),
            ),
        ]);
        assert!(out.contains("<rdf:Description rdf:nodeID=\"x1\">"));
        // b1 is taken, so the label that is not an NCName becomes b2.
        assert!(out.contains("<ns1:p rdf:nodeID=\"b2\"/>"));
        assert!(out.contains("<rdf:Description rdf:nodeID=\"b2\">"));
        assert!(out.contains("<ns1:p rdf:nodeID=\"b1\"/>"));
    }

    #[test]
    fn splits_predicates_on_the_longest_ncname() {
        assert_eq!(split("http://e/a#b").unwrap(), ("http://e/a#", "b"));
        assert_eq!(
            split("http://e/v1.2/x-y").unwrap(),
            ("http://e/v1.2/", "x-y")
        );
        assert_eq!(split("http://e/1a").unwrap(), ("http://e/1", "a"));
        assert_eq!(split("urn:x:é").unwrap(), ("urn:x:", "é"));
    }

    #[test]
    fn predicates_without_a_local_name_are_errors() {
        for p in ["http://e/", "http://e/123", "http://e/p/-", "abc"] {
            let e = check(&quad(Term::iri("http://e/s"), p, Term::iri("http://e/o"))).unwrap_err();
            assert!(matches!(e, Error::Data(_)), "{p}: {e}");
            assert!(e.to_string().contains("XML name"), "{p}: {e}");
        }
    }

    #[test]
    fn syntax_names_are_not_properties() {
        for local in ["li", "about", "Description"] {
            let p = format!("{RDF}{local}");
            assert!(matches!(
                check(&quad(Term::iri("http://e/s"), &p, Term::iri("http://e/o"))),
                Err(Error::Data(_))
            ));
        }
        assert!(
            check(&quad(
                Term::iri("http://e/s"),
                &format!("{RDF}_1"),
                Term::iri("http://e/o")
            ))
            .is_ok()
        );
    }

    #[test]
    fn named_graphs_and_quoted_triples_are_rejected() {
        let mut q = quad(
            Term::iri("http://e/s"),
            "http://e/p",
            Term::iri("http://e/o"),
        );
        q.graph = Some(Term::iri("http://e/g"));
        assert!(matches!(check(&q), Err(Error::Mapping(_))));
        let t = Term::Triple(Box::new(rmlrs_model::Triple {
            subject: Term::iri("http://e/s"),
            predicate: Term::iri("http://e/p"),
            object: Term::iri("http://e/o"),
        }));
        let q = quad(t, "http://e/p", Term::iri("http://e/o"));
        assert!(matches!(check(&q), Err(Error::Unsupported(_))));
    }

    #[test]
    fn characters_xml_cannot_hold_are_errors() {
        let q = quad(
            Term::iri("http://e/s"),
            "http://e/p",
            Term::Literal(Literal::simple("a\u{1}b")),
        );
        assert!(matches!(serialize(&[q], "UTF-8"), Err(Error::Data(_))));
    }
}
