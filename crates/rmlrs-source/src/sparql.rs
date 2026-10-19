// SPDX-License-Identifier: Apache-2.0
//! SPARQL query results as a logical source (RML-IO, `formats:SPARQL_Results_*`).
//!
//! One iteration is one solution of a SELECT query; a reference is a variable
//! name without `?`. An unbound variable gives no value. Every value is the
//! string form of the bound term (an IRI, a literal's lexical form, a blank
//! node as `_:label`), as in the CSV results format: the term map decides
//! which RDF term it becomes.
//!
//! Read here: CSV, TSV and JSON results. XML results are not supported.

use std::io::BufRead;
use std::sync::Arc;

use rmlrs_model::{Error, Result, SparqlResultsFormat};
use serde_json::Value;

use crate::{Iteration, Iterations, NaturalValue, ViewRow, csv};

/// Reads query results in `format` as iterations.
pub fn solutions<'a>(
    format: SparqlResultsFormat,
    mut reader: Box<dyn BufRead + 'a>,
) -> Result<Iterations<'a>> {
    match format {
        SparqlResultsFormat::Csv => {
            let rows = csv::Rows::new(reader)?;
            let keys = Arc::new(rows.header());
            Ok(Box::new(rows.map(move |r| {
                let r = r?;
                let values = keys
                    .iter()
                    .map(|k| r.get(k).filter(|v| !v.is_empty()).map(NaturalValue::string))
                    .collect();
                Ok(Iteration::View(ViewRow {
                    keys: keys.clone(),
                    values,
                }))
            })))
        }
        SparqlResultsFormat::Tsv => {
            let mut header = String::new();
            reader.read_line(&mut header).map_err(io_error)?;
            let keys: Vec<String> = header
                .trim_end_matches(['\r', '\n'])
                .split('\t')
                .map(|v| v.strip_prefix('?').unwrap_or(v).to_owned())
                .collect();
            let keys = Arc::new(keys);
            let lines = reader
                .lines()
                .filter(|l| !matches!(l, Ok(l) if l.trim_end_matches('\r').is_empty()));
            Ok(Box::new(lines.map(move |line| {
                let line = line.map_err(io_error)?;
                let fields: Vec<&str> = line.trim_end_matches('\r').split('\t').collect();
                if fields.len() != keys.len() {
                    return Err(Error::Source(format!(
                        "TSV results row has {} fields, header has {}",
                        fields.len(),
                        keys.len()
                    )));
                }
                let values = fields
                    .into_iter()
                    .map(|f| match f {
                        "" => Ok(None),
                        term => tsv_term(term).map(|v| Some(NaturalValue::string(v))),
                    })
                    .collect::<Result<_>>()?;
                Ok(Iteration::View(ViewRow {
                    keys: keys.clone(),
                    values,
                }))
            })))
        }
        SparqlResultsFormat::Json => {
            let doc: Value = serde_json::from_reader(reader)
                .map_err(|e| Error::Source(format!("SPARQL JSON results: {e}")))?;
            json_solutions(&doc)
        }
        SparqlResultsFormat::Xml => Err(Error::Unsupported("SPARQL XML results".into())),
    }
}

fn io_error(e: std::io::Error) -> Error {
    Error::Source(e.to_string())
}

/// The solutions of a JSON results document (`head.vars`, `results.bindings`).
fn json_solutions(doc: &Value) -> Result<Iterations<'static>> {
    let bad = |what: &str| Error::Source(format!("SPARQL JSON results: {what}"));
    if doc.get("boolean").is_some() {
        return Err(bad("an ASK result has no solutions to iterate"));
    }
    let keys: Vec<String> = doc
        .pointer("/head/vars")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no head.vars"))?
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| bad("a variable is not a string"))
        })
        .collect::<Result<_>>()?;
    let keys = Arc::new(keys);
    let bindings = doc
        .pointer("/results/bindings")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no results.bindings"))?;
    let mut out = Vec::with_capacity(bindings.len());
    for b in bindings {
        let values = keys
            .iter()
            .map(|k| match b.get(k) {
                None => Ok(None),
                Some(term) => {
                    let value = term.get("value");
                    match (term.get("type").and_then(Value::as_str), value) {
                        (Some("bnode"), Some(Value::String(v))) => {
                            Ok(Some(NaturalValue::string(format!("_:{v}"))))
                        }
                        (Some("uri" | "literal" | "typed-literal"), Some(Value::String(v))) => {
                            Ok(Some(NaturalValue::string(v.as_str())))
                        }
                        (Some("triple"), _) => Err(Error::Unsupported(
                            "quoted triples in SPARQL results".into(),
                        )),
                        _ => Err(bad(&format!("bad binding of {k}: {term}"))),
                    }
                }
            })
            .collect::<Result<_>>()?;
        out.push(Ok(Iteration::View(ViewRow {
            keys: keys.clone(),
            values,
        })));
    }
    Ok(Box::new(out.into_iter()))
}

/// The string form of an RDF term written as in SPARQL TSV results (Turtle
/// syntax): an IRI without its brackets, a literal's lexical form, a blank
/// node as is.
fn tsv_term(t: &str) -> Result<String> {
    let bad = || Error::Source(format!("bad term in TSV results: {t}"));
    if let Some(iri) = t.strip_prefix('<').and_then(|r| r.strip_suffix('>')) {
        return unescape(iri, false).ok_or_else(bad);
    }
    if t.starts_with("_:") {
        return Ok(t.to_owned());
    }
    if let Some(q) = t.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let long: String = std::iter::repeat_n(q, 3).collect();
        let (body, rest) = if let Some(r) = t.strip_prefix(long.as_str()) {
            let end = r.find(long.as_str()).ok_or_else(bad)?;
            (&r[..end], &r[end + 3..])
        } else {
            let r = &t[1..];
            let end = closing_quote(r, q).ok_or_else(bad)?;
            (&r[..end], &r[end + 1..])
        };
        // A language tag or a datatype may follow; the lexical form is all
        // that is kept.
        if !(rest.is_empty() || rest.starts_with('@') || rest.starts_with("^^")) {
            return Err(bad());
        }
        return unescape(body, true).ok_or_else(bad);
    }
    // Numbers and booleans in their short form.
    let number = t
        .trim_start_matches(['+', '-'])
        .chars()
        .all(|c| c.is_ascii_digit() || matches!(c, '.' | 'e' | 'E' | '+' | '-'));
    if t == "true" || t == "false" || (number && t.chars().any(|c| c.is_ascii_digit())) {
        return Ok(t.to_owned());
    }
    Err(bad())
}

/// The position of the quote that ends a short string, skipping escapes.
fn closing_quote(s: &str, q: char) -> Option<usize> {
    let mut escaped = false;
    for (i, c) in s.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            c if c == q => return Some(i),
            _ => {}
        }
    }
    None
}

/// Resolves `\uXXXX` and `\UXXXXXXXX` escapes, and in strings the other
/// Turtle escapes (`\t`, `\n`, `\"`, ...). `None` for a bad escape.
fn unescape(s: &str, string: bool) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        let e = chars.next()?;
        let hex = |n: usize, chars: &mut std::str::Chars<'_>| {
            let digits: String = chars.by_ref().take(n).collect();
            (digits.len() == n)
                .then(|| u32::from_str_radix(&digits, 16).ok())
                .flatten()
                .and_then(char::from_u32)
        };
        out.push(match e {
            'u' => hex(4, &mut chars)?,
            'U' => hex(8, &mut chars)?,
            't' if string => '\t',
            'b' if string => '\u{8}',
            'n' if string => '\n',
            'r' if string => '\r',
            'f' if string => '\u{c}',
            '"' | '\'' | '\\' if string => e,
            _ => return None,
        });
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The solutions as (variable, value) pairs, unbound variables left out.
    fn read(format: SparqlResultsFormat, text: &str) -> Result<Vec<Vec<(String, String)>>> {
        solutions(format, Box::new(text.as_bytes()))?
            .map(|it| match it? {
                Iteration::View(row) => Ok(row
                    .keys
                    .iter()
                    .zip(&row.values)
                    .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.lexical.clone())))
                    .collect()),
                other => panic!("not a solution: {other:?}"),
            })
            .collect()
    }

    fn pairs(p: &[(&str, &str)]) -> Vec<(String, String)> {
        p.iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn csv_results() {
        let text = "person,name\r\nhttp://e/0,\"Geller, Monica\"\r\nhttp://e/1,\r\n";
        assert_eq!(
            read(SparqlResultsFormat::Csv, text).unwrap(),
            [
                pairs(&[("person", "http://e/0"), ("name", "Geller, Monica")]),
                pairs(&[("person", "http://e/1")]),
            ]
        );
    }

    #[test]
    fn tsv_results() {
        let text = "?s\t?label\t?n\n<http://e/a>\t\"say \\\"hi\\\"\"@en\t42\n_:b0\t\"x\"^^<http://www.w3.org/2001/XMLSchema#string>\t\n<http://e/\\u00E9>\t'''long'''\t-1.5e3\n";
        assert_eq!(
            read(SparqlResultsFormat::Tsv, text).unwrap(),
            [
                pairs(&[("s", "http://e/a"), ("label", "say \"hi\""), ("n", "42")]),
                pairs(&[("s", "_:b0"), ("label", "x")]),
                pairs(&[("s", "http://e/é"), ("label", "long"), ("n", "-1.5e3")]),
            ]
        );
        for bad in [
            "?s\n<http://e/a\n",
            "?s\n\"open\n",
            "?s\nword\n",
            "?s\t?t\n<http://e/a>\n",
        ] {
            assert!(
                matches!(read(SparqlResultsFormat::Tsv, bad), Err(Error::Source(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn json_results() {
        let text = r#"{ "head": { "vars": ["s", "o"] },
            "results": { "bindings": [
              { "s": { "type": "uri", "value": "http://e/a" },
                "o": { "type": "literal", "value": "33", "datatype": "http://www.w3.org/2001/XMLSchema#integer" } },
              { "s": { "type": "bnode", "value": "b1" } }
            ] } }"#;
        assert_eq!(
            read(SparqlResultsFormat::Json, text).unwrap(),
            [
                pairs(&[("s", "http://e/a"), ("o", "33")]),
                pairs(&[("s", "_:b1")]),
            ]
        );
        assert!(matches!(
            read(
                SparqlResultsFormat::Json,
                r#"{ "head": {}, "boolean": true }"#
            ),
            Err(Error::Source(_))
        ));
        assert!(matches!(
            read(SparqlResultsFormat::Xml, "<sparql/>"),
            Err(Error::Unsupported(_))
        ));
    }
}
