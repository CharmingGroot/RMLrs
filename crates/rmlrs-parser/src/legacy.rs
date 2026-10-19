// SPDX-License-Identifier: Apache-2.0
//! Mappings in the legacy vocabularies: R2RML (`rr:`) and the RML that came
//! before the KG-Construct modules (`http://semweb.mmlab.be/ns/rml#`, with
//! `ql:` reference formulations). Their triples are translated to the
//! current RML vocabulary, and the mapping is then read like any other.
//!
//! Most terms keep their local name. The differences:
//!
//! - R2RML's `rr:logicalTable` becomes a logical source over the database
//!   given to the processor ([`PROCESSOR_DATABASE`]), as R2RML has no source
//!   description of its own.
//! - `rr:tableName` and `rr:sqlQuery` (old RML: `rml:query`) become the
//!   logical source's iterator with the `rml:SQL2008Table` /
//!   `rml:SQL2008Query` reference formulation. A query takes precedence over
//!   a table name given next to it, as in the old RML processor, and the
//!   `ql:CSV` old RML gave database sources is dropped.
//! - `rr:column` becomes `rml:reference`.
//! - `rr:sqlVersion` and `rr:inverseExpression` are dropped: neither changes
//!   the generated triples.
//! - Old RML wrote JSONPath references relative to the iteration (`"ID"`);
//!   current RML references start with `$`. In legacy documents, such
//!   references of JSON logical sources get `$.` in front
//!   ([`absolute_json_paths`]).
//! - Old RML read a SPARQL endpoint (`sd:Service` with `sd:endpoint`) by
//!   sending `rml:query` and iterating the JSON results with JSONPath
//!   (`$.results.bindings[*]`, references `NAME.value`). That becomes the
//!   RML-IO SPARQL source: the query is the iterator, the reference
//!   formulation `formats:SPARQL_Results_JSON`, and `NAME.value` (or `NAME`)
//!   the variable `NAME` ([`sparql_variables`]). Other old SPARQL sources
//!   (another iterator, result format or reference formulation) are
//!   reported as not supported ([`OLD_SPARQL_UNSUPPORTED`]).

use std::collections::HashSet;

use rmlrs_model::{
    Error, Expression, Mapping, ObjectMap, RDF_TYPE, ReferenceFormulation, Result, Source,
    SparqlResultsFormat, TemplatePart, Term, TermMap, Triple,
};

use crate::rml::RML;

const R2RML: &str = "http://www.w3.org/ns/r2rml#";
const OLD_RML: &str = "http://semweb.mmlab.be/ns/rml#";
const QL: &str = "http://semweb.mmlab.be/ns/ql#";

/// The source of an R2RML logical table: the database the processor is
/// given (`Options::database`), not a source described in the mapping.
pub const PROCESSOR_DATABASE: &str = "urn:x-rmlrs:r2rml-database";

/// The reference formulation an old SPARQL source gets when it is not the
/// JSON-results pattern RMLrs translates; the engine reports it as not
/// supported.
pub const OLD_SPARQL_UNSUPPORTED: &str = "urn:x-rmlrs:old-rml-sparql-source";

const SD_ENDPOINT: &str = "http://www.w3.org/ns/sparql-service-description#endpoint";
const SD_RESULT_FORMAT: &str = "http://www.w3.org/ns/sparql-service-description#resultFormat";
const SPARQL_JSON: &str = "http://www.w3.org/ns/formats/SPARQL_Results_JSON";
/// The JSONPath iterator old RML used over SPARQL JSON results.
const BINDINGS: &str = "$.results.bindings[*]";

/// Old RML logical sources that query a SPARQL endpoint, with the reference
/// formulation they get: `formats:SPARQL_Results_JSON` for the pattern
/// RMLrs translates, [`OLD_SPARQL_UNSUPPORTED`] otherwise.
fn old_sparql_sources(triples: &[Triple]) -> Vec<(Term, Term)> {
    let objects = |s: &Term, p: &str| -> Vec<&Term> {
        triples
            .iter()
            .filter(|t| &t.subject == s && t.predicate.as_iri() == Some(p))
            .map(|t| &t.object)
            .collect()
    };
    let old = |local: &str| format!("{OLD_RML}{local}");
    let mut out: Vec<(Term, Term)> = Vec::new();
    for t in triples {
        if t.predicate.as_iri() != Some(old("source").as_str())
            || objects(&t.object, SD_ENDPOINT).is_empty()
            || objects(&t.subject, &old("query")).is_empty()
            || out.iter().any(|(s, _)| s == &t.subject)
        {
            continue;
        }
        let ls = &t.subject;
        let jsonpath = format!("{QL}JSONPath");
        let json = matches!(
            objects(ls, &old("referenceFormulation")).as_slice(),
            [f] if f.as_iri() == Some(jsonpath.as_str())
        );
        let bindings = match objects(ls, &old("iterator")).as_slice() {
            [] => true,
            [Term::Literal(l)] => l.value.trim() == BINDINGS,
            _ => false,
        };
        let json_results = match objects(&t.object, SD_RESULT_FORMAT).as_slice() {
            [] => true,
            [f] => f.as_iri() == Some(SPARQL_JSON),
            _ => false,
        };
        let rf = if json && bindings && json_results {
            SPARQL_JSON
        } else {
            OLD_SPARQL_UNSUPPORTED
        };
        out.push((ls.clone(), Term::iri(rf)));
    }
    out
}

/// The local name of a term in a legacy namespace.
fn legacy_local(t: &Term) -> Option<(&'static str, &str)> {
    let iri = t.as_iri()?;
    [R2RML, OLD_RML, QL]
        .into_iter()
        .find_map(|ns| iri.strip_prefix(ns).map(|local| (ns, local)))
}

fn rml(local: &str) -> Term {
    Term::Iri(format!("{RML}{local}"))
}

/// A term of the legacy vocabularies in the current one; other terms as is.
fn current(t: Term) -> Term {
    match legacy_local(&t) {
        Some((_, "LogicalTable" | "BaseTableOrView" | "R2RMLView")) => rml("LogicalSource"),
        Some((_, local)) => rml(local),
        None => t,
    }
}

/// Whether any triple uses a legacy vocabulary.
pub fn uses_legacy(triples: &[Triple]) -> bool {
    triples
        .iter()
        .any(|t| legacy_local(&t.predicate).is_some() || legacy_local(&t.object).is_some())
}

/// Translates the triples of a mapping document to the current RML
/// vocabulary. Documents without legacy terms come back unchanged.
pub fn translate(triples: Vec<Triple>) -> Vec<Triple> {
    if !uses_legacy(&triples) {
        return triples;
    }
    let sparql = old_sparql_sources(&triples);
    let is_sparql = |s: &Term| sparql.iter().any(|(ls, _)| ls == s);
    let with_query: HashSet<Term> = triples
        .iter()
        .filter(|t| {
            matches!(
                legacy_local(&t.predicate),
                Some((R2RML, "sqlQuery") | (OLD_RML, "query"))
            )
        })
        .map(|t| t.subject.clone())
        .collect();
    let sql_sources: HashSet<Term> = triples
        .iter()
        .filter(|t| {
            matches!(
                legacy_local(&t.predicate),
                Some((_, "tableName" | "sqlQuery" | "query"))
            )
        })
        .map(|t| t.subject.clone())
        .collect();
    let mut out = Vec::with_capacity(triples.len());
    let mut tables: Vec<Term> = Vec::new();
    let mut with_source: HashSet<Term> = HashSet::new();
    let formulation = |s: &Term, local: &str| Triple {
        subject: s.clone(),
        predicate: rml("referenceFormulation"),
        object: rml(local),
    };
    for t in triples {
        let (s, p, o) = (t.subject, t.predicate, t.object);
        let predicate = match legacy_local(&p) {
            Some((_, "sqlVersion" | "inverseExpression")) => continue,
            // An old SPARQL source: the query is the iterator, and the
            // reference formulation is set below.
            Some((OLD_RML, "referenceFormulation" | "iterator")) if is_sparql(&s) => continue,
            Some((OLD_RML, "query")) if is_sparql(&s) => rml("iterator"),
            Some((R2RML, "logicalTable")) => {
                tables.push(o.clone());
                rml("logicalSource")
            }
            Some((_, "tableName")) if with_query.contains(&s) => continue,
            Some((OLD_RML, "referenceFormulation")) if sql_sources.contains(&s) => continue,
            Some((_, "tableName")) => {
                out.push(formulation(&s, "SQL2008Table"));
                rml("iterator")
            }
            Some((R2RML, "sqlQuery") | (OLD_RML, "query")) => {
                out.push(formulation(&s, "SQL2008Query"));
                rml("iterator")
            }
            Some((R2RML, "column")) => rml("reference"),
            _ => current(p),
        };
        if predicate == rml("source") {
            with_source.insert(s.clone());
        }
        let object = if predicate.as_iri() == Some(RDF_TYPE) || legacy_local(&o).is_some() {
            current(o)
        } else {
            o
        };
        out.push(Triple {
            subject: s,
            predicate,
            object,
        });
    }
    for (ls, rf) in sparql {
        out.push(Triple {
            subject: ls,
            predicate: rml("referenceFormulation"),
            object: rf,
        });
    }
    for table in tables {
        if !with_source.contains(&table) {
            out.push(Triple {
                subject: table,
                predicate: rml("source"),
                object: Term::iri(PROCESSOR_DATABASE),
            });
        }
    }
    out
}

/// The references of old RML SPARQL sources, written as JSONPath over the
/// JSON results (`"name.value"`, or `"name"` for the whole binding, which
/// old RML compared in join conditions), become the variable `name`. Any
/// other path into a binding (`"name.type"`, ...) is not supported.
pub fn sparql_variables(mapping: &mut Mapping) -> Result<()> {
    let sparql: Vec<bool> = mapping
        .triples_maps
        .iter()
        .map(|tm| {
            matches!(tm.logical_source.source, Source::SparqlEndpoint(_))
                && tm.logical_source.reference_formulation
                    == Some(ReferenceFormulation::SparqlResults(
                        SparqlResultsFormat::Json,
                    ))
        })
        .collect();
    let mut error = None;
    rewrite_references(mapping, &sparql, &mut |r| {
        let name = r.strip_suffix(".value").unwrap_or(r);
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            *r = name.to_owned();
        } else if error.is_none() {
            error = Some(Error::Unsupported(format!(
                "old RML reference {r:?} on SPARQL results (only NAME.value and NAME are read)"
            )));
        }
    });
    error.map_or(Ok(()), Err)
}

/// Applies `f` to every reference (also inside templates and join
/// conditions) of the triples maps for which `applies` is true. A join
/// condition's child side belongs to its triples map, its parent side to
/// the parent.
fn rewrite_references(mapping: &mut Mapping, applies: &[bool], f: &mut dyn FnMut(&mut String)) {
    for (i, tm) in mapping.triples_maps.iter_mut().enumerate() {
        let sm = &mut tm.subject_map;
        let mut maps: Vec<&mut TermMap> = vec![&mut sm.term_map];
        maps.extend(sm.graph_maps.iter_mut());
        for pom in &mut tm.predicate_object_maps {
            maps.extend(pom.predicate_maps.iter_mut());
            maps.extend(pom.graph_maps.iter_mut());
            for om in &mut pom.object_maps {
                match om {
                    ObjectMap::Term(m) => maps.push(m),
                    ObjectMap::Ref(r) => {
                        for jc in &mut r.join_conditions {
                            if applies[i] {
                                rewrite_expression(&mut jc.child, f);
                            }
                            if applies[r.parent] {
                                rewrite_expression(&mut jc.parent, f);
                            }
                        }
                    }
                }
            }
        }
        if applies[i] {
            for m in maps {
                rewrite_term_map(m, f);
            }
        }
    }
}

fn rewrite_term_map(m: &mut TermMap, f: &mut dyn FnMut(&mut String)) {
    if let Some(e) = &mut m.expression {
        rewrite_expression(e, f);
    }
    for inner in m.datatype_map.iter_mut().chain(m.language_map.iter_mut()) {
        rewrite_term_map(inner, f);
    }
}

fn rewrite_expression(e: &mut Expression, f: &mut dyn FnMut(&mut String)) {
    match e {
        Expression::Reference(r) => f(r),
        Expression::Template(t) => {
            for part in &mut t.parts {
                if let TemplatePart::Reference(r) = part {
                    f(r);
                }
            }
        }
        Expression::Constant(_) | Expression::Function(_) => {}
    }
}

/// Makes the relative JSONPath references of a legacy mapping absolute:
/// `"ID"` → `"$.ID"`, `"a b"` → `"$['a b']"`. Join conditions use the child's
/// formulation on the child side and the parent's on the parent side.
pub fn absolute_json_paths(mapping: &mut Mapping) {
    let json: Vec<bool> = mapping
        .triples_maps
        .iter()
        .map(|tm| tm.logical_source.reference_formulation == Some(ReferenceFormulation::JsonPath))
        .collect();
    rewrite_references(mapping, &json, &mut |r| *r = absolute(r));
}

fn absolute(reference: &str) -> String {
    if reference.starts_with('$') || reference.starts_with('@') {
        reference.to_owned()
    } else if !reference.is_empty()
        && reference.split('.').all(|s| {
            !s.is_empty()
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
    {
        format!("$.{reference}")
    } else {
        format!(
            "$['{}']",
            reference.replace('\\', "\\\\").replace('\'', "\\'")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turtle;

    fn translated(ttl: &str) -> Vec<String> {
        let prefixes = "@prefix rr: <http://www.w3.org/ns/r2rml#> . \
                        @prefix rml: <http://semweb.mmlab.be/ns/rml#> . \
                        @prefix ql: <http://semweb.mmlab.be/ns/ql#> . @prefix ex: <http://e/> .\n";
        let mut out: Vec<String> =
            translate(turtle::parse(&format!("{prefixes}{ttl}"), None).unwrap())
                .iter()
                .map(|t| format!("{} {} {}", t.subject, t.predicate, t.object))
                .collect();
        out.sort();
        out
    }

    #[test]
    fn r2rml_logical_tables_read_the_processor_database() {
        let out = translated(
            r#"ex:TM a rr:TriplesMap ; rr:logicalTable ex:LT .
               ex:LT rr:sqlQuery "SELECT 1" ; rr:sqlVersion rr:SQL2008 .
               ex:O rr:column "NAME" ; rr:termType rr:Literal ."#,
        );
        let n = |s: &str| format!("<http://w3id.org/rml/{s}>");
        assert_eq!(
            out,
            [
                format!("<http://e/LT> {} \"SELECT 1\"", n("iterator")),
                format!(
                    "<http://e/LT> {} {}",
                    n("referenceFormulation"),
                    n("SQL2008Query")
                ),
                format!("<http://e/LT> {} <{PROCESSOR_DATABASE}>", n("source")),
                format!("<http://e/O> {} \"NAME\"", n("reference")),
                format!("<http://e/O> {} {}", n("termType"), n("Literal")),
                format!("<http://e/TM> {} <http://e/LT>", n("logicalSource")),
                format!("<http://e/TM> <{RDF_TYPE}> {}", n("TriplesMap")),
            ]
        );
    }

    #[test]
    fn relative_json_paths() {
        assert_eq!(absolute("ID"), "$.ID");
        assert_eq!(absolute("a.b_c"), "$.a.b_c");
        assert_eq!(absolute("$.x"), "$.x");
        assert_eq!(absolute("Name with space"), "$['Name with space']");
        assert_eq!(absolute("it's"), "$['it\\'s']");
    }

    #[test]
    fn old_rml_sparql_sources() {
        let doc = |iterator: &str, refs: &str| {
            format!(
                r#"@prefix rr: <http://www.w3.org/ns/r2rml#> . @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
                   @prefix ql: <http://semweb.mmlab.be/ns/ql#> . @prefix ex: <http://e/> .
                   @prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
                   ex:S a sd:Service ; sd:endpoint <http://h/sparql> ;
                     sd:resultFormat <http://www.w3.org/ns/formats/SPARQL_Results_JSON> .
                   ex:TM rml:logicalSource [ rml:source ex:S ; rml:referenceFormulation ql:JSONPath ;
                       rml:iterator "{iterator}" ; rml:query "SELECT ?ID ?Name WHERE {{ ?x ?p ?o }}" ] ;
                     {refs} ."#
            )
        };
        let refs = r#"rr:subjectMap [ rr:template "http://e/{ID.value}" ] ;
                      rr:predicateObjectMap [ rr:predicate ex:p ; rr:objectMap [ rml:reference "Name.value" ] ] ;
                      rr:predicateObjectMap [ rr:predicate ex:q ; rr:objectMap [ rml:reference "Name" ] ]"#;
        let m = crate::parse_mapping(&doc("$.results.bindings[*]", refs), None).unwrap();
        let ls = &m.triples_maps[0].logical_source;
        assert_eq!(ls.source, Source::SparqlEndpoint("http://h/sparql".into()));
        assert_eq!(
            ls.reference_formulation,
            Some(ReferenceFormulation::SparqlResults(
                SparqlResultsFormat::Json
            ))
        );
        assert_eq!(
            ls.iterator.as_deref(),
            Some("SELECT ?ID ?Name WHERE { ?x ?p ?o }")
        );
        let tm = &m.triples_maps[0];
        assert_eq!(
            tm.subject_map.term_map.expression,
            Some(Expression::Template(rmlrs_model::Template {
                parts: vec![
                    TemplatePart::Text("http://e/".into()),
                    TemplatePart::Reference("ID".into())
                ]
            }))
        );
        let references: Vec<_> = tm
            .predicate_object_maps
            .iter()
            .map(|pom| match &pom.object_maps[0] {
                ObjectMap::Term(t) => t.expression.clone(),
                ObjectMap::Ref(_) => None,
            })
            .collect();
        assert_eq!(
            references,
            [
                Some(Expression::Reference("Name".into())),
                Some(Expression::Reference("Name".into()))
            ]
        );
        // Another path into a binding, or another iterator, is not supported.
        let other_path = refs.replace("\"Name\"", "\"Name.type\"");
        assert!(matches!(
            crate::parse_mapping(&doc("$.results.bindings[*]", &other_path), None),
            Err(Error::Unsupported(_))
        ));
        let m = crate::parse_mapping(&doc("$.results.bindings[0]", refs), None).unwrap();
        assert_eq!(
            m.triples_maps[0].logical_source.reference_formulation,
            Some(ReferenceFormulation::Other(format!(
                "<{OLD_SPARQL_UNSUPPORTED}>"
            )))
        );
    }

    #[test]
    fn old_rml_keeps_its_source() {
        let out = translated(
            r#"ex:LS rml:source "data.json" ; rml:referenceFormulation ql:JSONPath ; rml:iterator "$[*]" .
               ex:DB rml:source ex:db ; rr:tableName "student" ."#,
        );
        assert!(out.contains(&"<http://e/LS> <http://w3id.org/rml/referenceFormulation> <http://w3id.org/rml/JSONPath>".to_owned()));
        assert!(out.contains(&"<http://e/DB> <http://w3id.org/rml/referenceFormulation> <http://w3id.org/rml/SQL2008Table>".to_owned()));
        assert!(!out.iter().any(|l| l.contains(PROCESSOR_DATABASE)));
    }
}
