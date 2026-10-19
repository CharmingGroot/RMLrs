// SPDX-License-Identifier: Apache-2.0
//! Term generation: IRI-safe encoding, IRI checks, blank node labels and
//! literal validation, following the "term generation rules" of RML-Core.

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use rmlrs_model::{Error, Literal, Result, Term, TermType, XSD, iri};

/// RFC 3986 `unreserved` minus alphanumerics: these stay as they are.
const URI_UNRESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// The IRI-safe version of a string: every character outside RFC 3987
/// `iunreserved` is percent-encoded as UTF-8.
pub fn iri_safe(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~') || iri::is_ucschar(c) {
            out.push(c);
        } else {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        }
    }
    out
}

/// The URI-safe version of a string: every character outside RFC 3986
/// `unreserved` is percent-encoded as UTF-8.
pub fn uri_safe(value: &str) -> String {
    utf8_percent_encode(value, URI_UNRESERVED).to_string()
}

/// Applies the reference value transforming function of a template term map.
pub fn encode_for_template(value: &str, term_type: TermType) -> String {
    match term_type {
        TermType::Iri => iri_safe(value),
        TermType::Uri => uri_safe(value),
        _ => value.to_owned(),
    }
}

/// Turns a value into an IRI: as it is when absolute, otherwise prefixed with
/// the base IRI. Anything that is still not a valid absolute IRI is a data error.
pub fn make_iri(value: &str, base: &str) -> Result<Term> {
    if iri::is_valid_absolute_iri(value) {
        return Ok(Term::Iri(value.to_owned()));
    }
    let joined = format!("{base}{value}");
    if iri::is_valid_absolute_iri(&joined) {
        Ok(Term::Iri(joined))
    } else {
        Err(Error::Data(format!("{value:?} does not form a valid IRI")))
    }
}

/// `rml:UnsafeIRI` / `rml:UnsafeURI`: no percent-encoding and no validity
/// check beyond having a scheme. The official test RMLTC0027b expects
/// `<http://example.com/Person/Emily Smith>`, which is not a valid IRI, so
/// the "data error" rule of RML-Core section "Generated RDF term" is not
/// applied to the unsafe term types.
pub fn make_unsafe_iri(value: &str, base: &str) -> Term {
    if iri::is_absolute(value) {
        Term::Iri(value.to_owned())
    } else {
        Term::Iri(format!("{base}{value}"))
    }
}

/// A blank node whose label is derived from `value`, one-to-one, using only
/// characters that are valid in every RDF syntax.
pub fn blank_node(value: &str) -> Term {
    let mut label = String::from("b");
    for c in value.chars() {
        if c.is_ascii_alphanumeric() {
            label.push(c);
        } else {
            label.push_str(&format!("_{:X}_", c as u32));
        }
    }
    Term::BlankNode(label)
}

/// BCP 47, simplified: a 2-3 letter primary language subtag (or the `x`/`i`
/// singletons), followed by 1-8 character alphanumeric subtags.
pub fn is_valid_language_tag(tag: &str) -> bool {
    let mut parts = tag.split('-');
    let Some(first) = parts.next() else {
        return false;
    };
    // A primary language subtag of 4 to 8 letters is well-formed but never
    // valid: 4-letter subtags are reserved, and the IANA registry has no
    // 5- to 8-letter ones ("english" is not a language tag).
    let first_ok = match first.len() {
        1 => first.eq_ignore_ascii_case("x") || first.eq_ignore_ascii_case("i"),
        2..=3 => first.chars().all(|c| c.is_ascii_alphabetic()),
        _ => false,
    };
    first_ok
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// A literal with a datatype chosen by the mapping. Ill-typed literals of the
/// validatable XSD datatypes are data errors; well-typed ones get the XSD
/// canonical lexical form (RML-Core "natural RDF lexical form").
pub fn typed_literal(value: &str, datatype: &str) -> Result<Term> {
    if !is_well_typed(value, datatype) {
        return Err(Error::Data(format!("{value:?} is not a valid {datatype}")));
    }
    Ok(Term::Literal(Literal::typed(
        canonical(value, datatype),
        datatype,
    )))
}

/// XSD canonical forms for the numeric and boolean datatypes; other values
/// are kept as they are. Assumes `value` is well typed.
fn canonical(value: &str, datatype: &str) -> String {
    let Some(local) = datatype.strip_prefix(XSD) else {
        return value.to_owned();
    };
    match local {
        "integer" | "long" | "int" | "short" | "byte" | "nonNegativeInteger"
        | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" | "unsignedLong"
        | "unsignedInt" | "unsignedShort" | "unsignedByte" => {
            let neg = value.starts_with('-');
            let digits = value.trim_start_matches(['+', '-']).trim_start_matches('0');
            match (digits.is_empty(), neg) {
                (true, _) => "0".to_owned(),
                (false, true) => format!("-{digits}"),
                (false, false) => digits.to_owned(),
            }
        }
        "decimal" => {
            let neg = value.starts_with('-');
            let v = value.trim_start_matches(['+', '-']);
            let (int, frac) = v.split_once('.').unwrap_or((v, ""));
            let int = int.trim_start_matches('0');
            let frac = frac.trim_end_matches('0');
            let int = if int.is_empty() { "0" } else { int };
            let body = if frac.is_empty() {
                int.to_owned()
            } else {
                format!("{int}.{frac}")
            };
            if neg && body != "0" {
                format!("-{body}")
            } else {
                body
            }
        }
        "double" | "float" => match value {
            "INF" | "+INF" => "INF".to_owned(),
            "-INF" | "NaN" => value.to_owned(),
            _ => value
                .parse::<f64>()
                .map(canonical_double)
                .unwrap_or_else(|_| value.to_owned()),
        },
        "boolean" => match value {
            "1" | "true" => "true".to_owned(),
            _ => "false".to_owned(),
        },
        _ => value.to_owned(),
    }
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

fn is_well_typed(value: &str, datatype: &str) -> bool {
    let Some(local) = datatype.strip_prefix(XSD) else {
        return true;
    };
    let int = |v: &str| {
        let d = v.strip_prefix(['+', '-']).unwrap_or(v);
        !d.is_empty() && d.chars().all(|c| c.is_ascii_digit())
    };
    match local {
        "integer" | "long" | "int" | "short" | "byte" | "nonNegativeInteger"
        | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" | "unsignedLong"
        | "unsignedInt" | "unsignedShort" | "unsignedByte" => int(value),
        "decimal" => {
            let d = value.strip_prefix(['+', '-']).unwrap_or(value);
            let (a, b) = d.split_once('.').unwrap_or((d, ""));
            !(a.is_empty() && b.is_empty())
                && a.chars().all(|c| c.is_ascii_digit())
                && b.chars().all(|c| c.is_ascii_digit())
        }
        "double" | "float" => {
            matches!(value, "INF" | "+INF" | "-INF" | "NaN")
                || (value.parse::<f64>().is_ok() && !value.contains(['i', 'I', 'n', 'N']))
        }
        "boolean" => matches!(value, "true" | "false" | "1" | "0"),
        _ => true,
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn language_tags() {
        for ok in [
            "en",
            "en-US",
            "es-419",
            "zh-Hant-TW",
            "x-private",
            "i-klingon",
        ] {
            assert!(is_valid_language_tag(ok), "{ok}");
        }
        for bad in [
            "english",
            "a-english",
            "spanish",
            "e",
            "",
            "en--US",
            "en-toolongsubtag",
        ] {
            assert!(!is_valid_language_tag(bad), "{bad}");
        }
    }

    use super::*;

    #[test]
    fn spec_encoding_table() {
        for (s, iri, uri) in [
            ("42", "42", "42"),
            ("Hello World!", "Hello%20World%21", "Hello%20World%21"),
            (
                "2011-08-23T22:17:00Z",
                "2011-08-23T22%3A17%3A00Z",
                "2011-08-23T22%3A17%3A00Z",
            ),
            ("~A_17.1-2\u{a2}", "~A_17.1-2\u{a2}", "~A_17.1-2%C2%A2"),
            ("Zoë Krüger", "Zoë%20Krüger", "Zo%C3%AB%20Kr%C3%BCger"),
        ] {
            assert_eq!(iri_safe(s), iri, "{s}");
            assert_eq!(uri_safe(s), uri, "{s}");
        }
    }

    #[test]
    fn iri_from_value() {
        assert_eq!(
            make_iri("Venus", "http://example.com/").unwrap(),
            Term::iri("http://example.com/Venus")
        );
        assert!(make_iri("Juan Daniel", "http://example.com/").is_err());
    }

    #[test]
    fn blank_labels_are_injective() {
        assert_ne!(blank_node("a b"), blank_node("a_20_b"));
        assert_eq!(blank_node("BobSmith"), Term::BlankNode("bBobSmith".into()));
    }

    #[test]
    fn canonical_forms() {
        let x = |v: &str, t: &str| match typed_literal(v, &format!("{XSD}{t}")).unwrap() {
            Term::Literal(l) => l.value,
            _ => unreachable!(),
        };
        assert_eq!(x("33", "double"), "3.3E1");
        assert_eq!(x("+007", "integer"), "7");
        assert_eq!(x("-0", "int"), "0");
        assert_eq!(x("042.500", "decimal"), "42.5");
        assert_eq!(x("1", "boolean"), "true");
        assert_eq!(x("2023-10-01", "date"), "2023-10-01");
    }

    #[test]
    fn well_typed() {
        assert!(typed_literal("10", "http://www.w3.org/2001/XMLSchema#int").is_ok());
        assert!(typed_literal("X", "http://www.w3.org/2001/XMLSchema#boolean").is_err());
        assert!(typed_literal("abc", "http://example.com/custom").is_ok());
    }
}
