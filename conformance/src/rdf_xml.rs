// SPDX-License-Identifier: Apache-2.0
//! Reads RDF/XML into triples, for comparing target outputs.
//!
//! The subset covers what the test suite's expected files and RMLrs's own
//! writer use, and a little more: `rdf:RDF`, `rdf:Description` and typed node
//! elements with `rdf:about` or `rdf:nodeID` (or neither), property
//! attributes, property elements with `rdf:resource`, `rdf:nodeID`,
//! `rdf:datatype`, `xml:lang` (inherited), a nested node element, or
//! `rdf:parseType="Resource"`. Anything else (`xml:base` and relative IRIs,
//! `rdf:ID`, `rdf:li`, other parse types, DTDs) is an error rather than a
//! guess. The XML is tokenised by `xmlparser`; namespaces, character
//! references, and line-end and attribute-value normalisation are done here.

use std::collections::HashMap;

use rmlrs::{Literal, Quad, Term};
use xmlparser::{ElementEnd, Token, Tokenizer};

const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
const XML: &str = "http://www.w3.org/XML/1998/namespace";
const XMLNS: &str = "http://www.w3.org/2000/xmlns/";

#[derive(Debug)]
struct Element {
    /// Namespace IRI + local name.
    name: String,
    /// Attributes other than namespace declarations, by expanded name.
    attrs: Vec<(String, String)>,
    children: Vec<Child>,
}

#[derive(Debug)]
enum Child {
    Element(Element),
    Text(String),
}

pub fn parse(text: &str) -> Result<Vec<Quad>, String> {
    let root = tree(text)?;
    let mut r = Reader::default();
    if root.name == format!("{RDF}RDF") {
        let lang = lang_of(&root, None)?;
        for c in &root.children {
            match c {
                Child::Element(e) => {
                    r.node(e, lang.as_deref())?;
                }
                Child::Text(t) if t.trim().is_empty() => {}
                Child::Text(t) => return Err(format!("text in rdf:RDF: {t:?}")),
            }
        }
    } else {
        r.node(&root, None)?;
    }
    Ok(r.out)
}

#[derive(Default)]
struct Reader {
    out: Vec<Quad>,
    fresh: usize,
}

impl Reader {
    fn blank(&mut self) -> Term {
        self.fresh += 1;
        Term::BlankNode(format!("f{}", self.fresh))
    }

    fn emit(&mut self, subject: &Term, predicate: &str, object: Term) {
        self.out.push(Quad {
            subject: subject.clone(),
            predicate: Term::Iri(predicate.to_owned()),
            object,
            graph: None,
        });
    }

    /// A node element; returns its subject.
    fn node(&mut self, e: &Element, lang: Option<&str>) -> Result<Term, String> {
        let lang = lang_of(e, lang)?;
        let subject = match (attr(e, "about"), attr(e, "nodeID")) {
            (Some(a), None) => iri(a)?,
            (None, Some(n)) => Term::BlankNode(format!("d{n}")),
            (None, None) => self.blank(),
            (Some(_), Some(_)) => return Err("rdf:about and rdf:nodeID together".into()),
        };
        if e.name != format!("{RDF}Description") {
            self.emit(&subject, &format!("{RDF}type"), Term::Iri(e.name.clone()));
        }
        self.property_attrs(e, &subject, &["about", "nodeID"], lang.as_deref())?;
        self.properties(e, &subject, lang.as_deref())?;
        Ok(subject)
    }

    /// The property elements of a node element.
    fn properties(
        &mut self,
        e: &Element,
        subject: &Term,
        lang: Option<&str>,
    ) -> Result<(), String> {
        for c in &e.children {
            match c {
                Child::Element(p) => self.property(p, subject, lang)?,
                Child::Text(t) if t.trim().is_empty() => {}
                Child::Text(t) => return Err(format!("text between properties: {t:?}")),
            }
        }
        Ok(())
    }

    /// Attributes of a node element (or an empty property element) that are
    /// properties. `skip` lists the rdf: attributes already handled.
    fn property_attrs(
        &mut self,
        e: &Element,
        subject: &Term,
        skip: &[&str],
        lang: Option<&str>,
    ) -> Result<(), String> {
        for (name, value) in &e.attrs {
            if name.starts_with(XML) {
                continue;
            }
            if let Some(local) = name.strip_prefix(RDF)
                && skip.contains(&local)
            {
                continue;
            }
            check_property(name)?;
            let object = if *name == format!("{RDF}type") {
                iri(value)?
            } else {
                literal(value.clone(), lang, None)
            };
            self.emit(subject, name, object);
        }
        Ok(())
    }

    fn property(&mut self, p: &Element, subject: &Term, lang: Option<&str>) -> Result<(), String> {
        check_property(&p.name)?;
        let lang = lang_of(p, lang)?;
        let lang = lang.as_deref();
        let elements: Vec<&Element> = p
            .children
            .iter()
            .filter_map(|c| match c {
                Child::Element(e) => Some(e),
                Child::Text(_) => None,
            })
            .collect();
        let text: String = p
            .children
            .iter()
            .filter_map(|c| match c {
                Child::Text(t) => Some(t.as_str()),
                Child::Element(_) => None,
            })
            .collect();
        if attr(p, "ID").is_some() {
            return Err("rdf:ID is not supported (no base IRI)".into());
        }
        let resource = attr(p, "resource");
        let node_id = attr(p, "nodeID");
        let datatype = attr(p, "datatype");
        let object = match attr(p, "parseType") {
            Some("Resource") => {
                let o = self.blank();
                self.properties(p, &o, lang)?;
                o
            }
            Some(other) => return Err(format!("rdf:parseType={other:?} is not supported")),
            None if !elements.is_empty() => {
                if elements.len() > 1 || !text.trim().is_empty() || resource.is_some() {
                    return Err("a property element holds one node element".into());
                }
                self.node(elements[0], lang)?
            }
            None if resource.is_some() || node_id.is_some() || has_property_attrs(p) => {
                if !text.is_empty() || datatype.is_some() {
                    return Err("a resource property element is empty".into());
                }
                let o = match (resource, node_id) {
                    (Some(r), None) => iri(r)?,
                    (None, Some(n)) => Term::BlankNode(format!("d{n}")),
                    (None, None) => self.blank(),
                    (Some(_), Some(_)) => {
                        return Err("rdf:resource and rdf:nodeID together".into());
                    }
                };
                self.property_attrs(p, &o, &["resource", "nodeID"], lang)?;
                o
            }
            None => literal(text, lang, datatype),
        };
        self.emit(subject, &p.name, object);
        Ok(())
    }
}

fn has_property_attrs(e: &Element) -> bool {
    e.attrs.iter().any(|(n, _)| {
        !n.starts_with(XML)
            && !matches!(
                n.strip_prefix(RDF),
                Some("resource" | "nodeID" | "datatype" | "parseType" | "ID")
            )
    })
}

fn check_property(name: &str) -> Result<(), String> {
    match name.strip_prefix(RDF) {
        Some(
            "RDF" | "ID" | "about" | "parseType" | "resource" | "nodeID" | "datatype"
            | "Description" | "aboutEach" | "aboutEachPrefix" | "bagID",
        ) => Err(format!("<{name}> cannot be a property")),
        Some("li") => Err("rdf:li is not supported".into()),
        _ => Ok(()),
    }
}

fn literal(value: String, lang: Option<&str>, datatype: Option<&str>) -> Term {
    Term::Literal(match (datatype, lang) {
        (Some(d), _) => Literal::typed(value, d),
        (None, Some(l)) if !l.is_empty() => Literal::lang(value, l),
        _ => Literal::simple(value),
    })
}

fn iri(s: &str) -> Result<Term, String> {
    // Without xml:base, only absolute IRIs have a meaning here.
    match s.split_once(':') {
        Some((scheme, _))
            if !scheme.is_empty()
                && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
                && scheme
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) =>
        {
            Ok(Term::Iri(s.to_owned()))
        }
        _ => Err(format!("relative IRI {s:?} is not supported")),
    }
}

fn attr<'a>(e: &'a Element, rdf_local: &str) -> Option<&'a str> {
    e.attrs
        .iter()
        .find(|(n, _)| n.strip_prefix(RDF) == Some(rdf_local))
        .map(|(_, v)| v.as_str())
}

/// The `xml:lang` in scope for an element. `xml:base` is refused.
fn lang_of(e: &Element, inherited: Option<&str>) -> Result<Option<String>, String> {
    let mut lang = inherited.map(str::to_owned);
    for (n, v) in &e.attrs {
        match n.strip_prefix(XML) {
            Some("lang") => lang = Some(v.clone()),
            Some("base") => return Err("xml:base is not supported".into()),
            _ => {}
        }
    }
    Ok(lang)
}

/// Builds the element tree, resolving namespaces.
fn tree(text: &str) -> Result<Element, String> {
    struct Open {
        prefix: String,
        local: String,
        attrs: Vec<(String, String, String)>,
    }
    let mut scopes: Vec<HashMap<String, String>> =
        vec![HashMap::from([("xml".to_owned(), XML.to_owned())])];
    // Elements being read, with their prefixed names for the end tag.
    let mut stack: Vec<(Element, String)> = Vec::new();
    let mut pending: Option<Open> = None;
    let mut root = None;
    for token in Tokenizer::from(text) {
        match token.map_err(|e| e.to_string())? {
            Token::ElementStart { prefix, local, .. } => {
                pending = Some(Open {
                    prefix: prefix.as_str().to_owned(),
                    local: local.as_str().to_owned(),
                    attrs: Vec::new(),
                });
            }
            Token::Attribute {
                prefix,
                local,
                value,
                ..
            } => {
                let open = pending.as_mut().ok_or("attribute outside a start tag")?;
                open.attrs.push((
                    prefix.as_str().to_owned(),
                    local.as_str().to_owned(),
                    attribute_value(value.as_str())?,
                ));
            }
            Token::ElementEnd { end, .. } => {
                let element = match end {
                    ElementEnd::Open | ElementEnd::Empty => {
                        let open = pending.take().ok_or("end of a start tag without a start")?;
                        let mut scope = scopes.last().cloned().unwrap_or_default();
                        for (p, l, v) in &open.attrs {
                            match (p.as_str(), l.as_str()) {
                                ("xmlns", l) => {
                                    if l == "xml" || l == "xmlns" || v == XMLNS {
                                        return Err(format!("reserved namespace binding {l}"));
                                    }
                                    scope.insert(l.to_owned(), v.clone());
                                }
                                ("", "xmlns") => {
                                    scope.insert(String::new(), v.clone());
                                }
                                _ => {}
                            }
                        }
                        let resolve = |p: &str, l: &str, default: bool| -> Result<String, String> {
                            if p.is_empty() && !default {
                                return Err(format!("unqualified attribute {l}"));
                            }
                            match scope.get(p) {
                                Some(ns) => Ok(format!("{ns}{l}")),
                                None if p.is_empty() => {
                                    Err(format!("element {l} has no namespace"))
                                }
                                None => Err(format!("undeclared prefix {p}")),
                            }
                        };
                        let name = resolve(&open.prefix, &open.local, true)?;
                        let mut attrs = Vec::new();
                        for (p, l, v) in &open.attrs {
                            if p == "xmlns" || (p.is_empty() && l == "xmlns") {
                                continue;
                            }
                            attrs.push((resolve(p, l, false)?, v.clone()));
                        }
                        let element = Element {
                            name,
                            attrs,
                            children: Vec::new(),
                        };
                        let qname = if open.prefix.is_empty() {
                            open.local
                        } else {
                            format!("{}:{}", open.prefix, open.local)
                        };
                        if matches!(end, ElementEnd::Open) {
                            scopes.push(scope);
                            stack.push((element, qname));
                            continue;
                        }
                        element
                    }
                    ElementEnd::Close(prefix, local) => {
                        let (element, qname) = stack.pop().ok_or("end tag without a start tag")?;
                        let close = if prefix.as_str().is_empty() {
                            local.as_str().to_owned()
                        } else {
                            format!("{}:{}", prefix.as_str(), local.as_str())
                        };
                        if close != qname {
                            return Err(format!("<{qname}> closed by </{close}>"));
                        }
                        scopes.pop();
                        element
                    }
                };
                match stack.last_mut() {
                    Some((parent, _)) => parent.children.push(Child::Element(element)),
                    None if root.is_none() => root = Some(element),
                    None => return Err("more than one root element".into()),
                }
            }
            Token::Text { text } => {
                let t = text_value(text.as_str())?;
                match stack.last_mut() {
                    Some((parent, _)) => parent.children.push(Child::Text(t)),
                    None if t.trim().is_empty() => {}
                    None => return Err("text outside the root element".into()),
                }
            }
            Token::Cdata { text, .. } => {
                let (parent, _) = stack.last_mut().ok_or("CDATA outside the root element")?;
                parent
                    .children
                    .push(Child::Text(normalise_line_ends(text.as_str())));
            }
            Token::Declaration { .. }
            | Token::Comment { .. }
            | Token::ProcessingInstruction { .. } => {}
            Token::DtdStart { .. }
            | Token::EmptyDtd { .. }
            | Token::EntityDeclaration { .. }
            | Token::DtdEnd { .. } => return Err("DTDs are not supported".into()),
        }
    }
    if !stack.is_empty() {
        return Err("unclosed element".into());
    }
    root.ok_or_else(|| "no root element".into())
}

fn normalise_line_ends(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// Element text: line ends normalised, then references replaced.
fn text_value(raw: &str) -> Result<String, String> {
    references(&normalise_line_ends(raw))
}

/// Attribute values: white space characters become spaces, then references
/// are replaced (so `&#xA;` stays a newline).
fn attribute_value(raw: &str) -> Result<String, String> {
    let spaced: String = normalise_line_ends(raw)
        .chars()
        .map(|c| if matches!(c, '\t' | '\n') { ' ' } else { c })
        .collect();
    references(&spaced)
}

fn references(s: &str) -> Result<String, String> {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let end = rest[i..].find(';').ok_or("unterminated reference")? + i;
        let name = &rest[i + 1..end];
        let c = match name {
            "amp" => '&',
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            _ => {
                let code = if let Some(h) = name.strip_prefix("#x") {
                    u32::from_str_radix(h, 16).ok()
                } else if let Some(d) = name.strip_prefix('#') {
                    d.parse().ok()
                } else {
                    None
                };
                code.and_then(char::from_u32)
                    .ok_or_else(|| format!("unknown reference &{name};"))?
            }
        };
        out.push(c);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nq(quads: &[Quad]) -> String {
        let mut lines: Vec<String> = quads.iter().map(ToString::to_string).collect();
        lines.sort();
        lines.join("\n")
    }

    #[test]
    fn reads_the_suite_shape() {
        let quads = parse(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
  xmlns:foaf="http://xmlns.com/foaf/0.1/">
  <rdf:Description rdf:about="http://example.org/0">
    <foaf:age rdf:datatype="http://www.w3.org/2001/XMLSchema#integer">33</foaf:age>
    <foaf:name>Monica Geller</foaf:name>
  </rdf:Description>
</rdf:RDF>"#,
        )
        .unwrap();
        assert_eq!(
            nq(&quads),
            [
                "<http://example.org/0> <http://xmlns.com/foaf/0.1/age> \"33\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
                "<http://example.org/0> <http://xmlns.com/foaf/0.1/name> \"Monica Geller\" .",
            ]
            .join("\n")
        );
    }

    #[test]
    fn references_languages_and_nesting() {
        let quads = parse(
            r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:e="http://e/" xml:lang="en">
  <e:Person rdf:nodeID="a" e:nick="x&#xA;y" e:raw="x
y">
    <e:name>a &amp; b &lt;c&gt;&#xD;</e:name>
    <e:knows><rdf:Description rdf:about="http://e/b?x=1&amp;y=2"/></e:knows>
    <e:fr xml:lang="FR">chat</e:fr>
    <e:plain xml:lang="">x</e:plain>
  </e:Person>
</rdf:RDF>"#,
        )
        .unwrap();
        let text = nq(&quads);
        assert!(text.contains(
            "_:da <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://e/Person> ."
        ));
        assert!(text.contains("_:da <http://e/nick> \"x\\ny\"@en ."));
        assert!(text.contains("_:da <http://e/raw> \"x y\"@en ."));
        assert!(text.contains("_:da <http://e/name> \"a & b <c>\\r\"@en ."));
        assert!(text.contains("_:da <http://e/knows> <http://e/b?x=1&y=2> ."));
        assert!(text.contains("_:da <http://e/fr> \"chat\"@fr ."));
        assert!(text.contains("_:da <http://e/plain> \"x\" ."));
    }

    #[test]
    fn unsupported_features_are_errors() {
        let wrap = |body: &str| {
            format!(
                r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:e="http://e/">{body}</rdf:RDF>"#
            )
        };
        for body in [
            r#"<rdf:Description rdf:about="rel"><e:p>x</e:p></rdf:Description>"#,
            r#"<rdf:Description rdf:about="http://e/s"><rdf:li>x</rdf:li></rdf:Description>"#,
            r#"<rdf:Description rdf:about="http://e/s"><e:p rdf:parseType="Literal">x</e:p></rdf:Description>"#,
            r#"<rdf:Description xml:base="http://e/" rdf:about="s"/>"#,
            r#"<rdf:Description rdf:about="http://e/s"><e:p>x</e:q></rdf:Description>"#,
        ] {
            assert!(parse(&wrap(body)).is_err(), "{body}");
        }
    }
}
