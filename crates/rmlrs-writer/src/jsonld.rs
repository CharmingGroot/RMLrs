// SPDX-License-Identifier: Apache-2.0
//! JSON-LD 1.1 for logical targets, in flattened, expanded form: no
//! `@context`, full IRIs everywhere, so that the document means the same to
//! every JSON-LD processor without a context to fetch.
//!
//! The document is an array of node objects, one per subject of the default
//! graph. A named graph is the `@graph` of the node object named after it
//! (created when the graph name is not a subject itself). Every value is a
//! value object (`@value` with `@language` or `@type`; `xsd:string` is left
//! implicit) or a node reference (`@id`; blank nodes as `_:label`), never a
//! native JSON number or boolean, so lexical forms are kept exactly.
//! Quoted triples (RDF-star) are not supported.

use std::collections::{BTreeMap, BTreeSet};

use rmlrs_model::{Error, Quad, Result, Term, XSD_STRING};
use serde_json::{Map, Value, json};

/// Predicate IRI to values.
type Node = BTreeMap<String, Vec<Value>>;
/// Subject id to node.
type Graph = BTreeMap<String, Node>;

/// Rejects, as early as possible, a quad JSON-LD cannot hold.
pub(crate) fn check(q: &Quad) -> Result<()> {
    for t in [&q.subject, &q.object] {
        if let Term::Triple(_) = t {
            return Err(Error::Unsupported("quoted triples in JSON-LD".into()));
        }
    }
    if let Term::Literal(_) = q.subject {
        return Err(Error::Data("literal subject".into()));
    }
    if let Some(Term::Literal(_) | Term::Triple(_)) = q.graph {
        return Err(Error::Data("graph name is not an IRI or blank node".into()));
    }
    Ok(())
}

/// Writes the whole document.
pub(crate) fn serialize(quads: &[Quad]) -> Result<String> {
    let mut default = Graph::new();
    let mut named: BTreeMap<String, Graph> = BTreeMap::new();
    for q in quads {
        check(q)?;
        let graph = match &q.graph {
            None => &mut default,
            Some(g) => named.entry(id(g)).or_default(),
        };
        graph
            .entry(id(&q.subject))
            .or_default()
            .entry(q.predicate.as_iri().unwrap_or_default().to_owned())
            .or_default()
            .push(value(&q.object));
    }
    let ids: BTreeSet<&String> = default.keys().chain(named.keys()).collect();
    let doc: Vec<Value> = ids
        .into_iter()
        .map(|i| {
            let mut obj = node_object(i, default.get(i));
            if let Some(g) = named.get(i) {
                let nodes = g
                    .iter()
                    .map(|(s, n)| Value::Object(node_object(s, Some(n))))
                    .collect();
                obj.insert("@graph".into(), Value::Array(nodes));
            }
            Value::Object(obj)
        })
        .collect();
    Ok(serde_json::to_string_pretty(&doc).expect("JSON values serialise"))
}

fn node_object(id: &str, node: Option<&Node>) -> Map<String, Value> {
    let mut obj = Map::new();
    obj.insert("@id".into(), json!(id));
    for (p, values) in node.into_iter().flatten() {
        obj.insert(p.clone(), Value::Array(values.clone()));
    }
    obj
}

/// The `@id` of an IRI or blank node.
fn id(t: &Term) -> String {
    match t {
        Term::BlankNode(b) => format!("_:{b}"),
        Term::Iri(i) => i.clone(),
        _ => unreachable!("rejected by check"),
    }
}

fn value(t: &Term) -> Value {
    match t {
        Term::Literal(l) => {
            let mut v = Map::new();
            v.insert("@value".into(), json!(l.value));
            if let Some(lang) = &l.language {
                v.insert("@language".into(), json!(lang));
            } else if l.datatype != XSD_STRING {
                v.insert("@type".into(), json!(l.datatype));
            }
            Value::Object(v)
        }
        other => json!({ "@id": id(other) }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmlrs_model::Literal;

    fn quad(s: Term, p: &str, o: Term, g: Option<Term>) -> Quad {
        Quad {
            subject: s,
            predicate: Term::iri(p),
            object: o,
            graph: g,
        }
    }

    fn doc(quads: &[Quad]) -> Value {
        serde_json::from_str(&serialize(quads).unwrap()).unwrap()
    }

    #[test]
    fn literals_keep_their_lexical_form() {
        let d = doc(&[
            quad(
                Term::iri("http://e/s"),
                "http://e/p",
                Term::Literal(Literal::simple("a \"b\"\n")),
                None,
            ),
            quad(
                Term::iri("http://e/s"),
                "http://e/p",
                Term::Literal(Literal::typed(
                    "033",
                    "http://www.w3.org/2001/XMLSchema#integer",
                )),
                None,
            ),
            quad(
                Term::iri("http://e/s"),
                "http://e/p",
                Term::Literal(Literal::lang("chat", "fr")),
                None,
            ),
        ]);
        assert_eq!(
            d,
            json!([{
                "@id": "http://e/s",
                "http://e/p": [
                    { "@value": "a \"b\"\n" },
                    { "@value": "033", "@type": "http://www.w3.org/2001/XMLSchema#integer" },
                    { "@value": "chat", "@language": "fr" }
                ]
            }])
        );
    }

    #[test]
    fn named_graphs_hang_off_the_graph_node() {
        let d = doc(&[
            quad(
                Term::iri("http://e/g"),
                "http://e/p",
                Term::iri("http://e/o"),
                None,
            ),
            quad(
                Term::BlankNode("b0".into()),
                "http://e/p",
                Term::BlankNode("b1".into()),
                Some(Term::iri("http://e/g")),
            ),
            quad(
                Term::iri("http://e/s"),
                "http://e/p",
                Term::iri("http://e/o"),
                Some(Term::BlankNode("g2".into())),
            ),
        ]);
        assert_eq!(
            d,
            json!([
                { "@id": "_:g2", "@graph": [
                    { "@id": "http://e/s", "http://e/p": [{ "@id": "http://e/o" }] }
                ]},
                { "@id": "http://e/g", "http://e/p": [{ "@id": "http://e/o" }], "@graph": [
                    { "@id": "_:b0", "http://e/p": [{ "@id": "_:b1" }] }
                ]}
            ])
        );
    }

    #[test]
    fn empty_target_is_an_empty_array() {
        assert_eq!(serialize(&[]).unwrap(), "[]");
    }

    #[test]
    fn quoted_triples_are_not_supported() {
        let t = Term::Triple(Box::new(rmlrs_model::Triple {
            subject: Term::iri("http://e/s"),
            predicate: Term::iri("http://e/p"),
            object: Term::iri("http://e/o"),
        }));
        let q = quad(Term::iri("http://e/s"), "http://e/p", t, None);
        assert!(matches!(serialize(&[q]), Err(Error::Unsupported(_))));
    }
}
