// SPDX-License-Identifier: Apache-2.0
//! An N-Quads parser, used to read expected test outputs. Quoted triples
//! (`<< s p o >>`, N-Triples-star) are read too.

use rmlrs_model::{Error, Literal, Quad, Result, Term, Triple};

pub fn parse(input: &str) -> Result<Vec<Quad>> {
    let mut quads = Vec::new();
    for (n, line) in input.lines().enumerate() {
        let mut l = Line {
            s: line,
            pos: 0,
            line: n + 1,
        };
        l.ws();
        if l.done() || l.peek() == Some('#') {
            continue;
        }
        let subject = l.term()?;
        let predicate = l.term()?;
        let object = l.term()?;
        l.ws();
        let graph = if l.peek() == Some('.') {
            None
        } else {
            Some(l.term()?)
        };
        l.ws();
        if l.peek() != Some('.') {
            return l.err("expected '.'");
        }
        quads.push(Quad {
            subject,
            predicate,
            object,
            graph,
        });
    }
    Ok(quads)
}

struct Line<'a> {
    s: &'a str,
    pos: usize,
    line: usize,
}

impl Line<'_> {
    fn err<T>(&self, msg: &str) -> Result<T> {
        Err(Error::Syntax(format!("n-quads line {}: {msg}", self.line)))
    }

    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }

    fn done(&self) -> bool {
        self.pos >= self.s.len()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn ws(&mut self) {
        while self.peek().is_some_and(|c| c == ' ' || c == '\t') {
            self.pos += 1;
        }
    }

    fn term(&mut self) -> Result<Term> {
        self.ws();
        if self.s[self.pos..].starts_with("<<") {
            self.pos += 2;
            let subject = self.term()?;
            let predicate = self.term()?;
            let object = self.term()?;
            self.ws();
            if !self.s[self.pos..].starts_with(">>") {
                return self.err("expected '>>'");
            }
            self.pos += 2;
            return Ok(Term::Triple(Box::new(Triple {
                subject,
                predicate,
                object,
            })));
        }
        match self.peek() {
            Some('<') => Ok(Term::Iri(self.iri()?)),
            Some('_') => {
                self.pos += 2;
                let start = self.pos;
                while self.peek().is_some_and(|c| !c.is_whitespace()) {
                    self.pos += 1;
                }
                Ok(Term::BlankNode(self.s[start..self.pos].to_owned()))
            }
            Some('"') => {
                self.pos += 1;
                let mut value = String::new();
                loop {
                    match self.bump() {
                        None => return self.err("unterminated literal"),
                        Some('"') => break,
                        Some('\\') => value.push(self.escape()?),
                        Some(c) => value.push(c),
                    }
                }
                match self.peek() {
                    Some('@') => {
                        self.pos += 1;
                        let start = self.pos;
                        while self
                            .peek()
                            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-')
                        {
                            self.pos += 1;
                        }
                        Ok(Term::Literal(Literal::lang(
                            value,
                            &self.s[start..self.pos],
                        )))
                    }
                    Some('^') => {
                        self.pos += 2;
                        Ok(Term::Literal(Literal::typed(value, self.iri()?)))
                    }
                    _ => Ok(Term::Literal(Literal::simple(value))),
                }
            }
            _ => self.err("expected a term"),
        }
    }

    fn iri(&mut self) -> Result<String> {
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.bump() {
                None => return self.err("unterminated IRI"),
                Some('>') => return Ok(out),
                Some('\\') => out.push(self.escape()?),
                Some(c) => out.push(c),
            }
        }
    }

    fn escape(&mut self) -> Result<char> {
        let c = match self.bump() {
            Some('t') => '\t',
            Some('b') => '\u{8}',
            Some('n') => '\n',
            Some('r') => '\r',
            Some('f') => '\u{c}',
            Some('"') => '"',
            Some('\'') => '\'',
            Some('\\') => '\\',
            Some(u @ ('u' | 'U')) => {
                let len = if u == 'u' { 4 } else { 8 };
                let hex = self.s.get(self.pos..self.pos + len).unwrap_or("");
                self.pos += len;
                let code = u32::from_str_radix(hex, 16)
                    .map_err(|_| Error::Syntax("bad \\u escape".into()))?;
                return char::from_u32(code).ok_or_else(|| Error::Syntax("bad code point".into()));
            }
            _ => return self.err("bad escape"),
        };
        Ok(c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoted_triples() {
        let src = "<< << _:a <http://e/p> \"1\" >> <http://e/q> <http://e/o> >> <http://e/r> <http://e/z> .\n";
        let q = parse(src).unwrap();
        assert_eq!(q[0].to_string(), src.trim_end());
        let Term::Triple(outer) = &q[0].subject else {
            panic!("not a quoted triple");
        };
        assert!(matches!(&outer.subject, Term::Triple(inner) if inner.subject.is_blank()));
        assert!(
            parse("<< <http://e/s> <http://e/p> <http://e/o> <http://e/q> <http://e/o> .").is_err()
        );
    }

    #[test]
    fn roundtrip() {
        let src = "<http://e/a> <http://e/b> \"x\\\"y\"@EN <http://e/g> .\n_:b1 <http://e/p> \"1\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n";
        let q = parse(src).unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(
            q[0].to_string(),
            "<http://e/a> <http://e/b> \"x\\\"y\"@en <http://e/g> ."
        );
        assert!(q[1].subject.is_blank());
    }
}
