// SPDX-License-Identifier: Apache-2.0
//! Logical sources: reading input data, iterating over it, and evaluating
//! references against one iteration: JSON (JSONPath), CSV, SQL and XML
//! (XPath).

pub mod compression;
pub mod csv;
pub mod encoding;
#[cfg(feature = "http")]
pub mod http;
pub mod jsonpath;
pub mod sparql;
pub mod sql;
pub mod xml;

use std::io::{BufRead, Read};
use std::path::Path as FsPath;
use std::sync::Arc;

use rmlrs_model::{Error, Result, XSD_BOOLEAN, XSD_DOUBLE, XSD_INTEGER, XSD_STRING};
use serde_json::Value;

pub use jsonpath::Path as JsonPath;

/// A value taken from a source, already in its natural RDF form: the
/// canonical lexical form and the natural datatype.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NaturalValue {
    pub lexical: String,
    pub datatype: &'static str,
}

impl NaturalValue {
    pub fn string(s: impl Into<String>) -> Self {
        NaturalValue {
            lexical: s.into(),
            datatype: XSD_STRING,
        }
    }
}

/// Reads a JSON document and returns its iterations: the nodes selected by
/// `iterator`, or the whole document when there is no iterator.
pub fn json_iterations(path: &FsPath, iterator: Option<&str>) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Error::Source(format!("{}: {e}", path.display())))?;
    json_iterations_from_str(&text, iterator).map_err(|e| match e {
        Error::Source(m) => Error::Source(format!("{}: {m}", path.display())),
        other => other,
    })
}

/// Same as [`json_iterations`], for a document already in memory.
pub fn json_iterations_from_str(text: &str, iterator: Option<&str>) -> Result<Vec<Value>> {
    let iterator = iterator.map(JsonPath::parse).transpose()?;
    let doc: Value = serde_json::from_str(text).map_err(|e| Error::Source(e.to_string()))?;
    Ok(match iterator {
        Some(it) => it.select(&doc).into_iter().cloned().collect(),
        None => vec![doc],
    })
}

/// One logical iteration, in the shape of its source.
#[derive(Debug, Clone, PartialEq)]
pub enum Iteration {
    Json(Value),
    Csv(csv::Row),
    Sql(sql::Row),
    Xml(xml::Node),
    /// One iteration of a logical view (RML-LV).
    View(ViewRow),
}

/// One iteration of a logical view: a value, or none, for each of its
/// referenceable keys (field names and index keys).
#[derive(Debug, Clone, PartialEq)]
pub struct ViewRow {
    pub keys: Arc<Vec<String>>,
    pub values: Vec<Option<NaturalValue>>,
}

impl ViewRow {
    pub fn get(&self, key: &str) -> Option<&NaturalValue> {
        let i = self.keys.iter().position(|k| k == key)?;
        self.values[i].as_ref()
    }
}

/// A reference expression compiled for its reference formulation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reference {
    Json(JsonPath),
    Column(String),
    XPath(xml::XPath),
}

/// The source formats RMLrs reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Json,
    Csv,
    Sql,
    /// XML with XPath. The number tells namespace declarations apart: the
    /// same expression can mean different things under different prefixes.
    Xml(u32),
    /// A logical view (RML-LV): references name its keys.
    View,
}

impl Format {
    /// Compiles a reference. `namespaces` are the XPath namespace
    /// declarations of the logical source (ignored by other formats).
    pub fn compile(self, reference: &str, namespaces: &xml::Namespaces) -> Result<Reference> {
        match self {
            Format::Json => JsonPath::parse(reference).map(Reference::Json),
            Format::Csv | Format::Sql | Format::View => Ok(Reference::Column(reference.to_owned())),
            Format::Xml(_) => xml::compile(reference, namespaces).map(Reference::XPath),
        }
    }

    /// Splits a document into iterations.
    pub fn iterations(self, text: &str, iterator: Option<&str>) -> Result<Vec<Iteration>> {
        self.open(Box::new(text.as_bytes()), iterator, &[])?
            .collect()
    }

    /// Opens a source for iteration. CSV is read row by row; JSON and XML
    /// are parsed as a whole document and then iterated.
    pub fn open<'a>(
        self,
        mut reader: Box<dyn BufRead + 'a>,
        iterator: Option<&str>,
        namespaces: &xml::Namespaces,
    ) -> Result<Iterations<'a>> {
        match self {
            Format::Xml(set) => {
                let nodes = xml::iterations(reader, iterator, namespaces, set)?;
                Ok(Box::new(nodes.into_iter().map(|n| Ok(Iteration::Xml(n)))))
            }
            Format::Json => {
                let mut text = String::new();
                reader
                    .read_to_string(&mut text)
                    .map_err(|e| Error::Source(e.to_string()))?;
                let its = json_iterations_from_str(&text, iterator)?;
                Ok(Box::new(its.into_iter().map(|v| Ok(Iteration::Json(v)))))
            }
            Format::Csv => Ok(Box::new(
                csv::Rows::new(reader)?.map(|r| r.map(Iteration::Csv)),
            )),
            Format::Sql => Err(Error::Mapping(
                "SQL sources are databases, not documents".into(),
            )),
            Format::View => Err(Error::Mapping("logical views are not documents".into())),
        }
    }
}

/// A lazy sequence of iterations.
pub type Iterations<'a> = Box<dyn Iterator<Item = Result<Iteration>> + 'a>;

/// Evaluates a compiled reference against one iteration of any format.
pub fn evaluate_reference(
    iteration: &Iteration,
    reference: &Reference,
) -> Result<Vec<NaturalValue>> {
    match (iteration, reference) {
        (Iteration::Json(v), Reference::Json(p)) => evaluate(v, p),
        (Iteration::Sql(row), Reference::Column(c)) => {
            Ok(row.get(c)?.cloned().into_iter().collect())
        }
        (Iteration::Csv(row), Reference::Column(c)) => {
            Ok(row.get(c).map(NaturalValue::string).into_iter().collect())
        }
        (Iteration::Xml(node), Reference::XPath(p)) => xml::evaluate(node, p),
        (Iteration::View(row), Reference::Column(c)) => {
            Ok(row.get(c).cloned().into_iter().collect())
        }
        _ => Err(Error::Mapping(
            "reference does not match the source format".into(),
        )),
    }
}

/// Evaluates a compiled reference against one iteration. Null and missing
/// values produce nothing; arrays and objects are data errors, because they
/// have no natural RDF form.
pub fn evaluate(iteration: &Value, reference: &JsonPath) -> Result<Vec<NaturalValue>> {
    let mut out = Vec::new();
    for v in reference.select(iteration) {
        match v {
            Value::Null => {}
            Value::String(s) => out.push(NaturalValue::string(s.as_str())),
            Value::Bool(b) => out.push(NaturalValue {
                lexical: b.to_string(),
                datatype: XSD_BOOLEAN,
            }),
            Value::Number(n) => out.push(natural_number(&n.to_string())?),
            Value::Array(_) | Value::Object(_) => {
                return Err(Error::Data(format!(
                    "reference selects a non-scalar value: {v}"
                )));
            }
        }
    }
    Ok(out)
}

/// Like [`evaluate`], but every value is a plain string in its JSON text form
/// (`10`, `30.50`, `true`): old RML gave JSON values no natural datatype.
pub fn evaluate_plain(iteration: &Value, reference: &JsonPath) -> Result<Vec<NaturalValue>> {
    let mut out = Vec::new();
    for v in reference.select(iteration) {
        match v {
            Value::Null => {}
            Value::String(s) => out.push(NaturalValue::string(s.as_str())),
            Value::Bool(_) | Value::Number(_) => out.push(NaturalValue::string(v.to_string())),
            Value::Array(_) | Value::Object(_) => {
                return Err(Error::Data(format!(
                    "reference selects a non-scalar value: {v}"
                )));
            }
        }
    }
    Ok(out)
}

/// JSON numbers: integers become `xsd:integer`, everything else `xsd:double`,
/// both in XSD canonical form.
fn natural_number(text: &str) -> Result<NaturalValue> {
    if !text.contains(['.', 'e', 'E']) {
        let neg = text.starts_with('-');
        let digits = text.trim_start_matches('-').trim_start_matches('0');
        let lexical = match (digits.is_empty(), neg) {
            (true, _) => "0".to_owned(),
            (false, true) => format!("-{digits}"),
            (false, false) => digits.to_owned(),
        };
        return Ok(NaturalValue {
            lexical,
            datatype: XSD_INTEGER,
        });
    }
    let f: f64 = text
        .parse()
        .map_err(|_| Error::Data(format!("bad JSON number {text}")))?;
    Ok(NaturalValue {
        lexical: canonical_double(f),
        datatype: XSD_DOUBLE,
    })
}

/// XSD canonical form of a double: one non-zero digit before the point, at
/// least one after it, and an `E` exponent (`1.5E2`, `0.0E0`).
pub fn canonical_double(f: f64) -> String {
    if f.is_nan() {
        return "NaN".into();
    }
    if f.is_infinite() {
        return if f > 0.0 { "INF".into() } else { "-INF".into() };
    }
    if f == 0.0 {
        return if f.is_sign_negative() {
            "-0.0E0".into()
        } else {
            "0.0E0".into()
        };
    }
    let s = format!("{f:e}");
    let (mantissa, exp) = s.split_once('e').expect("exponent form");
    let mantissa = if mantissa.contains('.') {
        mantissa.to_owned()
    } else {
        format!("{mantissa}.0")
    };
    format!("{mantissa}E{exp}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn natural_values() {
        let it =
            json!({"ID": 10, "Name": "Venus", "ok": true, "x": null, "d": 1.50, "arr": [1, 2]});
        let ev = |p: &str| evaluate(&it, &JsonPath::parse(p).unwrap());
        assert_eq!(
            ev("$.ID").unwrap(),
            [NaturalValue {
                lexical: "10".into(),
                datatype: XSD_INTEGER
            }]
        );
        assert_eq!(ev("$.Name").unwrap(), [NaturalValue::string("Venus")]);
        assert_eq!(ev("$.ok").unwrap()[0].lexical, "true");
        assert!(ev("$.x").unwrap().is_empty());
        assert!(ev("$.missing").unwrap().is_empty());
        assert_eq!(ev("$.d").unwrap()[0].lexical, "1.5E0");
        assert!(ev("$.arr").is_err());
        assert_eq!(ev("$.arr[*]").unwrap().len(), 2);
    }

    #[test]
    fn doubles() {
        assert_eq!(canonical_double(150.0), "1.5E2");
        assert_eq!(canonical_double(0.001), "1.0E-3");
        assert_eq!(canonical_double(-2.25), "-2.25E0");
    }
}
