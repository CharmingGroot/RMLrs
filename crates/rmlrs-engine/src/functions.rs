// SPDX-License-Identifier: Apache-2.0
//! Functions for RML-FNML: the built-in implementations and their
//! descriptions (which parameters a function takes, which outputs it has).
//!
//! Function IRIs are open-ended, so RMLrs cannot know every function a
//! mapping may name. A call to a function, a parameter or an output with no
//! description here (or among the functions the caller registers through
//! [`crate::Options::functions`]) is a mapping error, as RML-FNML requires.
//! Descriptions are compiled in, not read from FnO documents: an FnO
//! description says what a function looks like, not how to run it.

use rmlrs_model::{Error, Result, XSD_BOOLEAN, XSD_INTEGER};
use rmlrs_source::NaturalValue;

/// A function the engine can execute.
#[derive(Debug, Clone, Copy)]
pub struct Function {
    /// The function IRI.
    pub iri: &'static str,
    /// Parameter IRIs, in the order `run` receives their values. Every
    /// parameter is required.
    pub parameters: &'static [&'static str],
    /// Output IRIs, in the order `run` returns their values.
    pub outputs: &'static [&'static str],
    /// Runs the function on one value per parameter. `None` in the result
    /// means the output is NULL, so no term is generated.
    pub run: fn(&[&str]) -> Result<Vec<Option<NaturalValue>>>,
}

macro_rules! iri {
    (grel: $local:literal) => {
        concat!("http://users.ugent.be/~bjdmeest/function/grel.ttl#", $local)
    };
    (idlab: $local:literal) => {
        concat!("https://w3id.org/imec/idlab/function#", $local)
    };
}

fn string(s: impl Into<String>) -> Result<Vec<Option<NaturalValue>>> {
    Ok(vec![Some(NaturalValue::string(s))])
}

fn natural(lexical: String, datatype: &'static str) -> Result<Vec<Option<NaturalValue>>> {
    Ok(vec![Some(NaturalValue { lexical, datatype })])
}

/// The functions the RML-FNML test cases use (GREL and IDLab function IRIs).
pub const BUILT_IN: &[Function] = &[
    Function {
        iri: iri!(grel: "toUpperCase"),
        parameters: &[iri!(grel: "valueParam")],
        outputs: &[iri!(grel: "stringOut")],
        run: |a| string(a[0].to_uppercase()),
    },
    Function {
        iri: iri!(grel: "string_length"),
        parameters: &[iri!(grel: "valueParam")],
        outputs: &[iri!(grel: "output_number")],
        run: |a| natural(a[0].chars().count().to_string(), XSD_INTEGER),
    },
    Function {
        iri: iri!(grel: "string_substring"),
        parameters: &[iri!(grel: "valueParam"), iri!(grel: "p_int_i_from")],
        outputs: &[iri!(grel: "stringOut")],
        run: substring,
    },
    Function {
        iri: iri!(grel: "string_replace"),
        parameters: &[
            iri!(grel: "valueParam"),
            iri!(grel: "param_find"),
            iri!(grel: "param_replace"),
        ],
        outputs: &[iri!(grel: "stringOut")],
        run: |a| string(a[0].replace(a[1], a[2])),
    },
    Function {
        iri: iri!(grel: "escape"),
        parameters: &[iri!(grel: "valueParam"), iri!(grel: "modeParam")],
        outputs: &[iri!(grel: "stringOut")],
        run: escape,
    },
    Function {
        iri: iri!(idlab: "alwaysReturnsABC"),
        parameters: &[],
        outputs: &[iri!(idlab: "_stringOut")],
        run: |_| string("ABC"),
    },
    Function {
        iri: iri!(idlab: "toUpperCaseURL"),
        parameters: &[iri!(idlab: "str")],
        outputs: &[iri!(idlab: "_stringOut")],
        run: |a| string(a[0].to_uppercase()),
    },
    Function {
        iri: iri!(idlab: "equal"),
        parameters: &[iri!(grel: "valueParam"), iri!(grel: "valueParam2")],
        outputs: &[iri!(idlab: "_boolOut")],
        run: |a| natural((a[0] == a[1]).to_string(), XSD_BOOLEAN),
    },
];

/// GREL `substring(s, from)`: characters from `from` on; a negative index
/// counts from the end. An index past the end gives NULL, the end itself
/// gives the empty string.
fn substring(a: &[&str]) -> Result<Vec<Option<NaturalValue>>> {
    let from: i64 = a[1]
        .trim()
        .parse()
        .map_err(|_| Error::Data(format!("substring index {:?} is not an integer", a[1])))?;
    let len = a[0].chars().count() as i64;
    let from = if from < 0 { from + len } else { from };
    if !(0..=len).contains(&from) {
        return Ok(vec![None]);
    }
    string(a[0].chars().skip(from as usize).collect::<String>())
}

/// GREL `escape(s, mode)` for the modes GREL defines.
fn escape(a: &[&str]) -> Result<Vec<Option<NaturalValue>>> {
    let s = a[0];
    let out = match a[1] {
        "html" | "xml" => {
            let mut o = String::with_capacity(s.len());
            for c in s.chars() {
                match c {
                    '&' => o.push_str("&amp;"),
                    '<' => o.push_str("&lt;"),
                    '>' => o.push_str("&gt;"),
                    '"' => o.push_str("&quot;"),
                    '\'' if a[1] == "xml" => o.push_str("&apos;"),
                    _ => o.push(c),
                }
            }
            o
        }
        "csv" => {
            if s.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_owned()
            }
        }
        "url" => {
            percent_encoding::utf8_percent_encode(s, percent_encoding::NON_ALPHANUMERIC).to_string()
        }
        "javascript" => {
            let mut o = String::with_capacity(s.len());
            for c in s.chars() {
                match c {
                    '\\' => o.push_str("\\\\"),
                    '"' => o.push_str("\\\""),
                    '\'' => o.push_str("\\'"),
                    '\n' => o.push_str("\\n"),
                    '\r' => o.push_str("\\r"),
                    '\t' => o.push_str("\\t"),
                    _ => o.push(c),
                }
            }
            o
        }
        other => return Err(Error::Data(format!("unknown escape mode {other:?}"))),
    };
    string(out)
}

/// Finds a function: the caller's functions first, then the built-in ones.
pub fn find<'a>(extra: &'a [Function], iri: &str) -> Option<&'a Function> {
    extra.iter().chain(BUILT_IN).find(|f| f.iri == iri)
}

/// Checks a call against the function's description and returns, for each
/// parameter of the function, the index of the input that gives its value,
/// and the index of the output to use.
pub fn bind(
    f: &Function,
    inputs: &[(String, rmlrs_model::Expression)],
    output: Option<&str>,
) -> Result<(Vec<usize>, usize)> {
    for (p, _) in inputs {
        if !f.parameters.contains(&p.as_str()) {
            return Err(Error::Mapping(format!(
                "function <{}> has no parameter <{p}>",
                f.iri
            )));
        }
    }
    let mut order = Vec::with_capacity(f.parameters.len());
    for p in f.parameters {
        let found: Vec<usize> = inputs
            .iter()
            .enumerate()
            .filter(|(_, (q, _))| q == p)
            .map(|(i, _)| i)
            .collect();
        match found.as_slice() {
            [i] => order.push(*i),
            [] => {
                return Err(Error::Mapping(format!(
                    "function <{}> needs parameter <{p}>",
                    f.iri
                )));
            }
            _ => {
                return Err(Error::Mapping(format!(
                    "parameter <{p}> of <{}> is given more than once",
                    f.iri
                )));
            }
        }
    }
    let out =
        match output {
            None if f.outputs.is_empty() => {
                return Err(Error::Mapping(format!(
                    "function <{}> has no output",
                    f.iri
                )));
            }
            None => 0,
            Some(o) => f.outputs.iter().position(|x| *x == o).ok_or_else(|| {
                Error::Mapping(format!("function <{}> has no output <{o}>", f.iri))
            })?,
        };
    Ok((order, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(iri: &str, args: &[&str]) -> Option<String> {
        let f = find(&[], iri).unwrap();
        (f.run)(args).unwrap().remove(0).map(|v| v.lexical)
    }

    #[test]
    fn substring_bounds() {
        let f = iri!(grel: "string_substring");
        assert_eq!(call(f, &["Venus", "0"]).as_deref(), Some("Venus"));
        assert_eq!(call(f, &["Venus", "5"]).as_deref(), Some(""));
        assert_eq!(call(f, &["Venus", "6"]), None);
        assert_eq!(call(f, &["Venus", "-2"]).as_deref(), Some("us"));
    }

    #[test]
    fn escape_modes() {
        let f = iri!(grel: "escape");
        assert_eq!(call(f, &["A&B<'", "html"]).as_deref(), Some("A&amp;B&lt;'"));
        assert_eq!(call(f, &["a,b", "csv"]).as_deref(), Some("\"a,b\""));
        assert_eq!(call(f, &["a b", "url"]).as_deref(), Some("a%20b"));
    }

    #[test]
    fn binding_checks_the_description() {
        use rmlrs_model::{Expression, Term};
        let f = find(&[], iri!(grel: "toUpperCase")).unwrap();
        let v = || Expression::Constant(Term::iri("x"));
        assert!(bind(f, &[(iri!(grel: "valueParam").into(), v())], None).is_ok());
        assert!(bind(f, &[("http://e/unknown".into(), v())], None).is_err());
        assert!(bind(f, &[], None).is_err());
        assert!(
            bind(
                f,
                &[(iri!(grel: "valueParam").into(), v())],
                Some("http://e/out")
            )
            .is_err()
        );
    }
}
