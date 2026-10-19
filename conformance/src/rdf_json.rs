// SPDX-License-Identifier: Apache-2.0
//! Reads RDF/JSON (W3C Note) into triples, for comparing target outputs.
//! The expected files of the test suite write some literal values as JSON
//! numbers; those are read as their text.

use rmlrs::{Quad, Term};
use serde_json::Value;

pub fn parse(text: &str) -> Result<Vec<Quad>, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let Value::Object(subjects) = doc else {
        return Err("RDF/JSON must be an object".into());
    };
    let mut out = Vec::new();
    for (s, preds) in subjects {
        let subject = node(&s);
        let Value::Object(preds) = preds else {
            return Err("predicates must be an object".into());
        };
        for (p, objects) in preds {
            let Value::Array(objects) = objects else {
                return Err("objects must be an array".into());
            };
            for o in objects {
                out.push(Quad {
                    subject: subject.clone(),
                    predicate: Term::Iri(p.clone()),
                    object: object(&o)?,
                    graph: None,
                });
            }
        }
    }
    Ok(out)
}

fn node(s: &str) -> Term {
    match s.strip_prefix("_:") {
        Some(b) => Term::BlankNode(b.to_owned()),
        None => Term::Iri(s.to_owned()),
    }
}

fn object(o: &Value) -> Result<Term, String> {
    let text = |v: &Value| match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let value = o.get("value").map(text).ok_or("object without value")?;
    match o.get("type").and_then(Value::as_str) {
        Some("uri") => Ok(Term::Iri(value)),
        Some("bnode") => Ok(node(&value)),
        Some("literal") => Ok(Term::Literal(match (o.get("lang"), o.get("datatype")) {
            (Some(l), _) => rmlrs::Literal::lang(value, text(l)),
            (None, Some(d)) => rmlrs::Literal::typed(value, text(d)),
            (None, None) => rmlrs::Literal::simple(value),
        })),
        other => Err(format!("unknown object type {other:?}")),
    }
}
