// SPDX-License-Identifier: Apache-2.0
//! `rml:template` string templates.
//!
//! Unescaped `{` and `}` enclose a reference. `\{`, `\}` and `\\` stand for
//! the literal characters, inside and outside references alike. Any other
//! backslash, an unbalanced brace, or a brace nested inside a reference makes
//! the template invalid.

use rmlrs_model::{Error, Result, Template, TemplatePart};

pub fn parse(s: &str) -> Result<Template> {
    let mut parts = Vec::new();
    let mut text = String::new();
    let mut reference: Option<String> = None;
    let mut chars = s.chars();
    let bad = |why: &str| Err(Error::Mapping(format!("invalid template {s:?}: {why}")));

    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(e @ ('{' | '}' | '\\')) => match &mut reference {
                    Some(r) => r.push(e),
                    None => text.push(e),
                },
                _ => return bad("backslash must escape '{', '}' or '\\'"),
            },
            '{' => {
                if reference.is_some() {
                    return bad("nested '{'");
                }
                if !text.is_empty() {
                    parts.push(TemplatePart::Text(std::mem::take(&mut text)));
                }
                reference = Some(String::new());
            }
            '}' => match reference.take() {
                Some(r) if !r.is_empty() => parts.push(TemplatePart::Reference(r)),
                Some(_) => return bad("empty reference"),
                None => return bad("unmatched '}'"),
            },
            c => match &mut reference {
                Some(r) => r.push(c),
                None => text.push(c),
            },
        }
    }
    if reference.is_some() {
        return bad("unclosed '{'");
    }
    if !text.is_empty() {
        parts.push(TemplatePart::Text(text));
    }
    Ok(Template { parts })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(t: &Template) -> Vec<&str> {
        t.parts
            .iter()
            .filter_map(|p| match p {
                TemplatePart::Reference(r) => Some(r.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn basic() {
        let t = parse("http://example.com/{$.ID}/{$.Name}").unwrap();
        assert_eq!(refs(&t), ["$.ID", "$.Name"]);
    }

    #[test]
    fn escapes() {
        let t = parse(r"\{\{\{ {$['ISO 3166']} \}\}\}").unwrap();
        assert_eq!(t.parts[0], TemplatePart::Text("{{{ ".into()));
        assert_eq!(refs(&t), ["$['ISO 3166']"]);
        let t = parse(r"http://example.com/{$['\{Name\}']}").unwrap();
        assert_eq!(refs(&t), ["$['{Name}']"]);
    }

    #[test]
    fn invalid() {
        for s in [
            "http://example.com/{{Name}}",
            r"{\\{Name\\}}",
            "a}",
            "{a",
            "{}",
            r"\a",
        ] {
            assert!(parse(s).is_err(), "{s}");
        }
    }
}
