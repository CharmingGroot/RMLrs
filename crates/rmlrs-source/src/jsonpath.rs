// SPDX-License-Identifier: Apache-2.0
//! JSONPath (RFC 9535), the subset RML mappings use: the root `$`, member
//! names in dot and bracket notation, indices, slices, wildcards and the
//! descendant segment `..`. Filter expressions are reported as unsupported.

use rmlrs_model::{Error, Result};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Segment {
    descendant: bool,
    selectors: Vec<Selector>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Selector {
    Name(String),
    Wildcard,
    Index(i64),
    Slice(Option<i64>, Option<i64>, Option<i64>),
}

impl Path {
    pub fn parse(src: &str) -> Result<Path> {
        let p = PathParser {
            s: src,
            chars: src.char_indices().peekable(),
            segments: Vec::new(),
        };
        p.parse()
    }

    /// The nodes selected from `root`, in document order.
    pub fn select<'v>(&self, root: &'v Value) -> Vec<&'v Value> {
        let mut nodes = vec![root];
        for seg in &self.segments {
            let mut next = Vec::new();
            for n in nodes {
                if seg.descendant {
                    let mut all = Vec::new();
                    descendants(n, &mut all);
                    for d in all {
                        apply(&seg.selectors, d, &mut next);
                    }
                } else {
                    apply(&seg.selectors, n, &mut next);
                }
            }
            nodes = next;
        }
        nodes
    }
}

fn descendants<'v>(v: &'v Value, out: &mut Vec<&'v Value>) {
    out.push(v);
    match v {
        Value::Array(a) => a.iter().for_each(|c| descendants(c, out)),
        Value::Object(o) => o.values().for_each(|c| descendants(c, out)),
        _ => {}
    }
}

fn apply<'v>(selectors: &[Selector], v: &'v Value, out: &mut Vec<&'v Value>) {
    for sel in selectors {
        match (sel, v) {
            (Selector::Name(n), Value::Object(o)) => out.extend(o.get(n)),
            (Selector::Wildcard, Value::Object(o)) => out.extend(o.values()),
            (Selector::Wildcard, Value::Array(a)) => out.extend(a.iter()),
            (Selector::Index(i), Value::Array(a)) => {
                let i = if *i < 0 { a.len() as i64 + i } else { *i };
                if i >= 0 {
                    out.extend(a.get(i as usize));
                }
            }
            (Selector::Slice(start, end, step), Value::Array(a)) => {
                slice(a, *start, *end, *step, out)
            }
            _ => {}
        }
    }
}

fn slice<'v>(
    a: &'v [Value],
    start: Option<i64>,
    end: Option<i64>,
    step: Option<i64>,
    out: &mut Vec<&'v Value>,
) {
    let len = a.len() as i64;
    let step = step.unwrap_or(1);
    if step == 0 {
        return;
    }
    let norm = |i: i64| if i >= 0 { i } else { len + i };
    if step > 0 {
        let lo = start.map_or(0, norm).clamp(0, len);
        let hi = end.map_or(len, norm).clamp(0, len);
        let mut i = lo;
        while i < hi {
            out.push(&a[i as usize]);
            i += step;
        }
    } else {
        let hi = start.map_or(len - 1, norm).clamp(-1, len - 1);
        let lo = end.map_or(-len - 1, norm).clamp(-1, len - 1);
        let mut i = hi;
        while lo < i {
            out.push(&a[i as usize]);
            i += step;
        }
    }
}

struct PathParser<'a> {
    s: &'a str,
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    segments: Vec<Segment>,
}

impl PathParser<'_> {
    fn err<T>(&self, msg: &str) -> Result<T> {
        Err(Error::Mapping(format!(
            "invalid JSONPath {:?}: {msg}",
            self.s
        )))
    }

    fn peek(&mut self) -> Option<char> {
        self.chars.peek().map(|(_, c)| *c)
    }

    fn bump(&mut self) -> Option<char> {
        self.chars.next().map(|(_, c)| c)
    }

    fn parse(mut self) -> Result<Path> {
        if self.bump() != Some('$') {
            return self.err("must start with '$'");
        }
        while let Some(c) = self.peek() {
            match c {
                '.' => {
                    self.bump();
                    let descendant = self.peek() == Some('.');
                    if descendant {
                        self.bump();
                        if self.peek() == Some('[') {
                            let selectors = self.bracket()?;
                            self.segments.push(Segment {
                                descendant,
                                selectors,
                            });
                            continue;
                        }
                    }
                    let sel = if self.peek() == Some('*') {
                        self.bump();
                        Selector::Wildcard
                    } else {
                        Selector::Name(self.member_name()?)
                    };
                    self.segments.push(Segment {
                        descendant,
                        selectors: vec![sel],
                    });
                }
                '[' => {
                    let selectors = self.bracket()?;
                    self.segments.push(Segment {
                        descendant: false,
                        selectors,
                    });
                }
                _ => return self.err(&format!("unexpected '{c}'")),
            }
        }
        Ok(Path {
            segments: self.segments,
        })
    }

    fn member_name(&mut self) -> Result<String> {
        let mut name = String::new();
        while let Some(c) = self.peek() {
            if c.is_alphanumeric() || c == '_' || (c as u32) >= 0x80 {
                name.push(c);
                self.bump();
            } else {
                break;
            }
        }
        if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
            return self.err("expected a member name");
        }
        Ok(name)
    }

    fn ws(&mut self) {
        while self
            .peek()
            .is_some_and(|c| matches!(c, ' ' | '\t' | '\n' | '\r'))
        {
            self.bump();
        }
    }

    fn bracket(&mut self) -> Result<Vec<Selector>> {
        self.bump(); // '['
        let mut selectors = Vec::new();
        loop {
            self.ws();
            selectors.push(self.selector()?);
            self.ws();
            match self.bump() {
                Some(',') => continue,
                Some(']') => return Ok(selectors),
                _ => return self.err("expected ',' or ']'"),
            }
        }
    }

    fn selector(&mut self) -> Result<Selector> {
        match self.peek() {
            Some('*') => {
                self.bump();
                Ok(Selector::Wildcard)
            }
            Some(q @ ('\'' | '"')) => {
                self.bump();
                Ok(Selector::Name(self.quoted(q)?))
            }
            Some('?') => Err(Error::Unsupported(format!(
                "JSONPath filter expressions in {:?}",
                self.s
            ))),
            Some(c) if c == '-' || c == ':' || c.is_ascii_digit() => {
                let a = self.int()?;
                self.ws();
                if self.peek() != Some(':') {
                    return a
                        .map(Selector::Index)
                        .map_or_else(|| self.err("expected an index"), Ok);
                }
                self.bump();
                self.ws();
                let b = self.int()?;
                self.ws();
                let step = if self.peek() == Some(':') {
                    self.bump();
                    self.ws();
                    self.int()?
                } else {
                    None
                };
                Ok(Selector::Slice(a, b, step))
            }
            _ => self.err("expected a selector"),
        }
    }

    fn int(&mut self) -> Result<Option<i64>> {
        let mut s = String::new();
        if self.peek() == Some('-') {
            s.push('-');
            self.bump();
        }
        while let Some(c) = self.peek().filter(char::is_ascii_digit) {
            s.push(c);
            self.bump();
        }
        if s.is_empty() {
            return Ok(None);
        }
        s.parse().map(Some).or_else(|_| self.err("bad integer"))
    }

    fn quoted(&mut self, q: char) -> Result<String> {
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return self.err("unterminated string"),
                Some(c) if c == q => return Ok(out),
                Some('\\') => match self.bump() {
                    Some('b') => out.push('\u{8}'),
                    Some('f') => out.push('\u{c}'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    Some('t') => out.push('\t'),
                    Some('/') => out.push('/'),
                    Some('\\') => out.push('\\'),
                    Some('\'') => out.push('\''),
                    Some('"') => out.push('"'),
                    Some('u') => {
                        let hex: String = (0..4).filter_map(|_| self.bump()).collect();
                        let code = u32::from_str_radix(&hex, 16)
                            .or_else(|_| self.err("bad \\u escape"))?;
                        out.push(
                            char::from_u32(code)
                                .ok_or_else(|| Error::Mapping("bad code point".into()))?,
                        );
                    }
                    _ => return self.err("bad escape"),
                },
                Some(c) => out.push(c),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sel(path: &str, v: &Value) -> Vec<Value> {
        Path::parse(path)
            .unwrap()
            .select(v)
            .into_iter()
            .cloned()
            .collect()
    }

    #[test]
    fn basics() {
        let v = json!({"students": [{"ID": 10, "Name": "Venus"}, {"ID": 20, "Name": "Fernando"}], "ISO 3166": "BO"});
        assert_eq!(sel("$.students[*]", &v).len(), 2);
        assert_eq!(sel("$.students[0].Name", &v), [json!("Venus")]);
        assert_eq!(sel("$['ISO 3166']", &v), [json!("BO")]);
        assert_eq!(sel("$..Name", &v), [json!("Venus"), json!("Fernando")]);
        assert_eq!(sel("$.students[-1].ID", &v), [json!(20)]);
        assert_eq!(sel("$.students[0:1].ID", &v), [json!(10)]);
        assert_eq!(sel("$[*]", &json!([1, 2])), [json!(1), json!(2)]);
    }

    #[test]
    fn invalid() {
        for p in ["$.students[*]]", "students", "$.", "$[", "$['a"] {
            assert!(matches!(Path::parse(p), Err(Error::Mapping(_))), "{p}");
        }
        assert!(matches!(
            Path::parse("$[?(@.a)]"),
            Err(Error::Unsupported(_))
        ));
    }
}
