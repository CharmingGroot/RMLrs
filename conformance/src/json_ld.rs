// SPDX-License-Identifier: Apache-2.0
//! Reads JSON-LD into quads (the JSON-LD 1.1 "deserialize to RDF" result),
//! for comparing target outputs.
//!
//! The subset covers what the test suite's expected files and RMLrs's own
//! writer use, and a little more: inline contexts with prefixes, simple
//! terms, `@vocab`, `@language`, and term definitions with `@id`, `@type`
//! and `@language`; node objects (nested or not) with `@id`, `@type`,
//! `@graph` (named graphs), value objects, node references, and native
//! strings, numbers and booleans. Anything else (remote contexts, `@base`
//! and relative IRIs, `@list`, `@set`, `@reverse`, containers, ...) is an
//! error rather than a guess.

use std::collections::HashMap;

use rmlrs::{Literal, Quad, Term};
use serde_json::{Map, Value};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

#[derive(Debug, Clone, Default)]
struct Context {
    terms: HashMap<String, Definition>,
    vocab: Option<String>,
    language: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Definition {
    iri: String,
    /// `@id`, `@vocab`, or a datatype IRI.
    coerce: Option<String>,
    /// `Some(None)` when the term resets the default language.
    language: Option<Option<String>>,
}

pub fn parse(text: &str) -> Result<Vec<Quad>, String> {
    let doc: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut r = Reader::default();
    let root = Context::default();
    match &doc {
        Value::Array(items) => {
            for item in items {
                r.top(item, &root)?;
            }
        }
        other => r.top(other, &root)?,
    }
    Ok(r.out)
}

#[derive(Default)]
struct Reader {
    out: Vec<Quad>,
    fresh: usize,
}

impl Reader {
    /// A top-level object: a node, or `{"@context", "@graph"}` holding the
    /// nodes of the default graph.
    fn top(&mut self, v: &Value, ctx: &Context) -> Result<(), String> {
        let obj = v.as_object().ok_or("top-level items must be objects")?;
        let ctx = ctx.with(obj.get("@context"))?;
        if obj.contains_key("@graph") && obj.keys().all(|k| k == "@context" || k == "@graph") {
            for item in as_array(&obj["@graph"]) {
                let node = item.as_object().ok_or("@graph items must be objects")?;
                self.node(node, &ctx, None)?;
            }
        } else {
            self.node(obj, &ctx, None)?;
        }
        Ok(())
    }

    /// A node object; returns its subject.
    fn node(
        &mut self,
        obj: &Map<String, Value>,
        ctx: &Context,
        graph: Option<&Term>,
    ) -> Result<Term, String> {
        let ctx = ctx.with(obj.get("@context"))?;
        let subject = match obj.get("@id") {
            Some(Value::String(s)) => ctx.node_id(s)?,
            Some(other) => return Err(format!("@id must be a string, got {other}")),
            None => {
                self.fresh += 1;
                Term::BlankNode(format!("f{}", self.fresh))
            }
        };
        for (key, value) in obj {
            match key.as_str() {
                "@context" | "@id" | "@index" => {}
                "@type" => {
                    for t in as_array(value) {
                        let t = t.as_str().ok_or("@type values must be strings")?;
                        let object = ctx.expand(t, true)?;
                        self.emit(&subject, RDF_TYPE.to_owned(), object, graph);
                    }
                }
                "@graph" => {
                    for item in as_array(value) {
                        let node = item.as_object().ok_or("@graph items must be objects")?;
                        self.node(node, &ctx, Some(&subject))?;
                    }
                }
                k if k.starts_with('@') => return Err(format!("{k} is not supported")),
                k => {
                    let (predicate, def) = ctx.property(k)?;
                    for item in as_array(value) {
                        if let Some(object) = self.object(item, &ctx, &def, graph)? {
                            self.emit(&subject, predicate.clone(), object, graph);
                        }
                    }
                }
            }
        }
        Ok(subject)
    }

    fn object(
        &mut self,
        v: &Value,
        ctx: &Context,
        def: &Definition,
        graph: Option<&Term>,
    ) -> Result<Option<Term>, String> {
        let language = || match &def.language {
            Some(l) => l.clone(),
            None => ctx.language.clone(),
        };
        Ok(Some(match v {
            Value::Null => return Ok(None),
            Value::String(s) => match def.coerce.as_deref() {
                Some("@id") => ctx.node_id(s)?,
                Some("@vocab") => ctx.expand(s, true)?,
                Some(datatype) => Term::Literal(Literal::typed(s.clone(), datatype)),
                None => Term::Literal(match language() {
                    Some(l) => Literal::lang(s.clone(), l),
                    None => Literal::simple(s.clone()),
                }),
            },
            Value::Number(_) | Value::Bool(_) => {
                if def.coerce.is_some() {
                    return Err("type-coerced native values are not supported".into());
                }
                native(v)?
            }
            Value::Array(_) => return Err("nested arrays are not supported".into()),
            Value::Object(o) if o.contains_key("@value") => value_object(o, ctx)?,
            Value::Object(o) => {
                if let Some(k) = o.keys().find(|k| {
                    matches!(
                        k.as_str(),
                        "@list" | "@set" | "@reverse" | "@included" | "@nest"
                    )
                }) {
                    return Err(format!("{k} is not supported"));
                }
                self.node(o, ctx, graph)?
            }
        }))
    }

    fn emit(&mut self, subject: &Term, predicate: String, object: Term, graph: Option<&Term>) {
        self.out.push(Quad {
            subject: subject.clone(),
            predicate: Term::Iri(predicate),
            object,
            graph: graph.cloned(),
        });
    }
}

fn as_array(v: &Value) -> &[Value] {
    match v {
        Value::Array(a) => a,
        other => std::slice::from_ref(other),
    }
}

/// A native JSON number or boolean, as JSON-LD turns it into a literal.
fn native(v: &Value) -> Result<Term, String> {
    Ok(Term::Literal(match v {
        Value::Bool(b) => Literal::typed(b.to_string(), format!("{XSD}boolean")),
        Value::Number(n) => {
            let text = n.to_string();
            if text.contains(['.', 'e', 'E']) {
                let f: f64 = text.parse().map_err(|_| format!("bad number {text}"))?;
                Literal::typed(canonical_double(f), format!("{XSD}double"))
            } else {
                Literal::typed(text, format!("{XSD}integer"))
            }
        }
        other => return Err(format!("not a native value: {other}")),
    }))
}

/// The canonical `xsd:double` form JSON-LD uses (`1.5E1`).
fn canonical_double(f: f64) -> String {
    let s = format!("{f:E}");
    match s.split_once('E') {
        Some((m, e)) if !m.contains('.') => format!("{m}.0E{e}"),
        _ => s,
    }
}

fn value_object(o: &Map<String, Value>, ctx: &Context) -> Result<Term, String> {
    for k in o.keys() {
        if !matches!(k.as_str(), "@value" | "@language" | "@type" | "@index") {
            return Err(format!("{k} in a value object is not supported"));
        }
    }
    let datatype = match o.get("@type") {
        None => None,
        Some(Value::String(t)) => match ctx.expand(t, true)? {
            Term::Iri(i) => Some(i),
            _ => return Err("a datatype must be an IRI".into()),
        },
        Some(other) => return Err(format!("@type must be a string, got {other}")),
    };
    let language = match o.get("@language") {
        None | Some(Value::Null) => None,
        Some(Value::String(l)) => Some(l.clone()),
        Some(other) => return Err(format!("@language must be a string, got {other}")),
    };
    Ok(match (&o["@value"], datatype, language) {
        (Value::String(s), Some(d), None) => Term::Literal(Literal::typed(s.clone(), d)),
        (Value::String(s), None, Some(l)) => Term::Literal(Literal::lang(s.clone(), l)),
        (Value::String(s), None, None) => Term::Literal(Literal::simple(s.clone())),
        (v @ (Value::Number(_) | Value::Bool(_)), Some(d), None) => {
            let text = match v {
                Value::Number(n) => n.to_string(),
                other => other.to_string(),
            };
            Term::Literal(Literal::typed(text, d))
        }
        (v @ (Value::Number(_) | Value::Bool(_)), None, None) => native(v)?,
        _ => {
            return Err(format!(
                "unsupported value object {}",
                Value::Object(o.clone())
            ));
        }
    })
}

impl Context {
    /// Applies a local context.
    fn with(&self, local: Option<&Value>) -> Result<Context, String> {
        let mut ctx = self.clone();
        let Some(local) = local else {
            return Ok(ctx);
        };
        for c in as_array(local) {
            let obj = match c {
                Value::Null => {
                    ctx = Context::default();
                    continue;
                }
                Value::Object(o) => o,
                other => return Err(format!("context {other} is not supported")),
            };
            let mut defined = Vec::new();
            for (key, value) in obj {
                match (key.as_str(), value) {
                    ("@version", _) => {}
                    ("@vocab", Value::String(v)) => ctx.vocab = Some(v.clone()),
                    ("@vocab", Value::Null) => ctx.vocab = None,
                    ("@language", Value::String(l)) => ctx.language = Some(l.clone()),
                    ("@language", Value::Null) => ctx.language = None,
                    (k, _) if k.starts_with('@') => {
                        return Err(format!("{k} in a context is not supported"));
                    }
                    (k, Value::Null) => {
                        ctx.terms.remove(k);
                    }
                    (k, Value::String(iri)) => {
                        ctx.terms.insert(
                            k.to_owned(),
                            Definition {
                                iri: iri.clone(),
                                ..Default::default()
                            },
                        );
                        defined.push(k.to_owned());
                    }
                    (k, Value::Object(d)) => {
                        let mut def = Definition::default();
                        for (dk, dv) in d {
                            match (dk.as_str(), dv) {
                                ("@id", Value::String(i)) => def.iri = i.clone(),
                                ("@type", Value::String(t)) => def.coerce = Some(t.clone()),
                                ("@language", Value::String(l)) => {
                                    def.language = Some(Some(l.clone()));
                                }
                                ("@language", Value::Null) => def.language = Some(None),
                                _ => {
                                    return Err(format!(
                                        "{dk} in a term definition is not supported"
                                    ));
                                }
                            }
                        }
                        if def.iri.is_empty() {
                            def.iri = k.to_owned();
                        }
                        ctx.terms.insert(k.to_owned(), def);
                        defined.push(k.to_owned());
                    }
                    (k, other) => return Err(format!("term {k}: {other} is not supported")),
                }
            }
            // Term IRIs and datatypes may be compact IRIs over other terms.
            for k in defined {
                let mut def = ctx.terms[&k].clone();
                let mut without = ctx.clone();
                without.terms.remove(&k);
                def.iri = match without.expand(&def.iri, true)? {
                    Term::Iri(i) => i,
                    _ => return Err(format!("term {k} is not an IRI")),
                };
                if let Some(t) = &def.coerce
                    && !t.starts_with('@')
                {
                    def.coerce = match without.expand(t, true)? {
                        Term::Iri(i) => Some(i),
                        _ => return Err(format!("datatype of {k} is not an IRI")),
                    };
                }
                ctx.terms.insert(k, def);
            }
        }
        Ok(ctx)
    }

    /// IRI expansion. `vocab` is true for properties and types, which may be
    /// terms or relative to `@vocab`; `@id` values may not.
    fn expand(&self, s: &str, vocab: bool) -> Result<Term, String> {
        if vocab && let Some(def) = self.terms.get(s) {
            return Ok(Term::Iri(def.iri.clone()));
        }
        if let Some((prefix, suffix)) = s.split_once(':') {
            if prefix == "_" {
                return Ok(Term::BlankNode(format!("d{suffix}")));
            }
            if !suffix.starts_with("//")
                && let Some(def) = self.terms.get(prefix)
            {
                return Ok(Term::Iri(format!("{}{suffix}", def.iri)));
            }
            return Ok(Term::Iri(s.to_owned()));
        }
        match &self.vocab {
            Some(v) if vocab => Ok(Term::Iri(format!("{v}{s}"))),
            _ => Err(format!("relative IRI {s:?} is not supported")),
        }
    }

    fn node_id(&self, s: &str) -> Result<Term, String> {
        self.expand(s, false)
    }

    /// The IRI of a property key, with its term definition.
    fn property(&self, key: &str) -> Result<(String, Definition), String> {
        let def = self.terms.get(key).cloned().unwrap_or_default();
        match self.expand(key, true)? {
            Term::Iri(i) => Ok((i, def)),
            _ => Err(format!("property {key} is a blank node")),
        }
    }
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
            r#"{
  "@context": { "foaf": "http://xmlns.com/foaf/0.1/" },
  "@graph": [ {
      "@id": "http://example.org/0",
      "foaf:name": "Monica Geller",
      "foaf:age": { "@value": "33", "@type": "http://www.w3.org/2001/XMLSchema#integer" }
  } ]
}"#,
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
    fn named_graphs_terms_and_natives() {
        let quads = parse(
            r#"{
  "@context": { "e": "http://e/", "@vocab": "http://v/", "@language": "en", "http": "http://wrong/",
                "knows": { "@id": "e:knows", "@type": "@id" },
                "n": { "@id": "e:n", "@type": "e:dt" } },
  "@id": "e:g",
  "@type": "Thing",
  "@graph": [
    { "@id": "_:a", "knows": "e:b", "label": "hi", "n": "1",
      "i": 5, "d": 1.5, "b": true, "x": { "@value": "y", "@language": "FR" },
      "nested": { "e:p": { "@id": "e:c" }, "http://e/abs": "z" } }
  ]
}"#,
        )
        .unwrap();
        let text = nq(&quads);
        let g = "<http://e/g>";
        for line in [
            "<http://e/g> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://v/Thing> ."
                .to_owned(),
            format!("_:da <http://e/knows> <http://e/b> {g} ."),
            format!("_:da <http://v/label> \"hi\"@en {g} ."),
            format!("_:da <http://e/n> \"1\"^^<http://e/dt> {g} ."),
            format!("_:da <http://v/i> \"5\"^^<http://www.w3.org/2001/XMLSchema#integer> {g} ."),
            format!("_:da <http://v/d> \"1.5E0\"^^<http://www.w3.org/2001/XMLSchema#double> {g} ."),
            format!("_:da <http://v/b> \"true\"^^<http://www.w3.org/2001/XMLSchema#boolean> {g} ."),
            format!("_:da <http://v/x> \"y\"@fr {g} ."),
            format!("_:da <http://v/nested> _:f1 {g} ."),
            format!("_:f1 <http://e/p> <http://e/c> {g} ."),
            format!("_:f1 <http://e/abs> \"z\"@en {g} ."),
        ] {
            assert!(text.contains(&line), "missing {line} in\n{text}");
        }
        assert_eq!(quads.len(), 11);
    }

    #[test]
    fn unsupported_features_are_errors() {
        for doc in [
            r#"{ "@context": "http://e/context.jsonld", "@id": "http://e/s" }"#,
            r#"{ "@id": "rel", "http://e/p": "x" }"#,
            r#"{ "@id": "http://e/s", "http://e/p": { "@list": ["x"] } }"#,
            r#"{ "@context": { "@base": "http://e/" }, "@id": "s" }"#,
        ] {
            assert!(parse(doc).is_err(), "{doc}");
        }
    }

    #[test]
    fn canonical_doubles() {
        assert_eq!(canonical_double(1.0), "1.0E0");
        assert_eq!(canonical_double(15.0), "1.5E1");
        assert_eq!(canonical_double(-0.25), "-2.5E-1");
    }
}
