// SPDX-License-Identifier: Apache-2.0
//! A Turtle 1.1 parser, enough for RML mapping documents.
//!
//! It covers prefixes and base, `a`, predicate-object lists (`;`, `,`),
//! blank node property lists (`[ ... ]`), collections (`( ... )`), all four
//! string forms, numbers, booleans, language tags and datatypes.

use std::collections::HashMap;

use rmlrs_model::{
    Error, Literal, RDF_FIRST, RDF_NIL, RDF_REST, RDF_TYPE, Result, Term, Triple, XSD,
};

use rmlrs_model::iri;

/// Parses a Turtle document. `base` is the base IRI for relative IRIs.
pub fn parse(input: &str, base: Option<&str>) -> Result<Vec<Triple>> {
    let mut p = Parser {
        src: input.as_bytes(),
        text: input,
        pos: 0,
        base: base.map(str::to_owned),
        prefixes: HashMap::new(),
        triples: Vec::new(),
        blank_counter: 0,
    };
    p.document()?;
    Ok(p.triples)
}

struct Parser<'a> {
    src: &'a [u8],
    text: &'a str,
    pos: usize,
    base: Option<String>,
    prefixes: HashMap<String, String>,
    triples: Vec<Triple>,
    blank_counter: usize,
}

impl Parser<'_> {
    fn err<T>(&self, msg: &str) -> Result<T> {
        let line = self.text[..self.pos.min(self.text.len())]
            .matches('\n')
            .count()
            + 1;
        Err(Error::Syntax(format!("turtle line {line}: {msg}")))
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn peek_at(&self, off: usize) -> Option<u8> {
        self.src.get(self.pos + off).copied()
    }

    fn starts_with(&self, s: &str) -> bool {
        self.src[self.pos..].starts_with(s.as_bytes())
    }

    fn starts_with_keyword_ci(&self, kw: &str) -> bool {
        let end = self.pos + kw.len();
        end <= self.src.len()
            && self.src[self.pos..end].eq_ignore_ascii_case(kw.as_bytes())
            && self
                .src
                .get(end)
                .is_none_or(|c| c.is_ascii_whitespace() || *c == b'<')
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c.is_ascii_whitespace() {
                self.pos += 1;
            } else if c == b'#' {
                while let Some(c) = self.peek() {
                    self.pos += 1;
                    if c == b'\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn expect(&mut self, c: u8) -> Result<()> {
        self.skip_ws();
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            self.err(&format!("expected '{}'", c as char))
        }
    }

    fn fresh_blank(&mut self) -> Term {
        self.blank_counter += 1;
        Term::BlankNode(format!("anon{}", self.blank_counter))
    }

    fn emit(&mut self, subject: Term, predicate: Term, object: Term) {
        self.triples.push(Triple {
            subject,
            predicate,
            object,
        });
    }

    fn document(&mut self) -> Result<()> {
        loop {
            self.skip_ws();
            if self.peek().is_none() {
                return Ok(());
            }
            if self.starts_with("@prefix") {
                self.pos += 7;
                self.prefix_decl()?;
                self.expect(b'.')?;
            } else if self.starts_with("@base") {
                self.pos += 5;
                self.base_decl()?;
                self.expect(b'.')?;
            } else if self.starts_with_keyword_ci("PREFIX") {
                self.pos += 6;
                self.prefix_decl()?;
            } else if self.starts_with_keyword_ci("BASE") {
                self.pos += 4;
                self.base_decl()?;
            } else {
                self.triples_statement()?;
                self.expect(b'.')?;
            }
        }
    }

    fn prefix_decl(&mut self) -> Result<()> {
        self.skip_ws();
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == b':' {
                break;
            }
            if c.is_ascii_whitespace() {
                return self.err("expected ':' in prefix declaration");
            }
            self.pos += 1;
        }
        let name = self.text[start..self.pos].to_owned();
        self.pos += 1; // ':'
        self.skip_ws();
        let iri = self.iriref()?;
        self.prefixes.insert(name, iri);
        Ok(())
    }

    fn base_decl(&mut self) -> Result<()> {
        self.skip_ws();
        let iri = self.iriref()?;
        self.base = Some(iri);
        Ok(())
    }

    fn triples_statement(&mut self) -> Result<()> {
        self.skip_ws();
        if self.peek() == Some(b'[') {
            let subject = self.blank_node_property_list()?;
            self.skip_ws();
            if self.peek() != Some(b'.') {
                self.predicate_object_list(&subject)?;
            }
            return Ok(());
        }
        let subject = self.subject()?;
        self.predicate_object_list(&subject)
    }

    fn subject(&mut self) -> Result<Term> {
        self.skip_ws();
        match self.peek() {
            Some(b'<') => Ok(Term::Iri(self.iriref()?)),
            Some(b'_') if self.peek_at(1) == Some(b':') => self.blank_label(),
            Some(b'(') => self.collection(),
            Some(_) => Ok(Term::Iri(self.prefixed_name()?)),
            None => self.err("unexpected end of input"),
        }
    }

    fn predicate_object_list(&mut self, subject: &Term) -> Result<()> {
        loop {
            self.skip_ws();
            let predicate = self.verb()?;
            self.object_list(subject, &predicate)?;
            self.skip_ws();
            if self.peek() != Some(b';') {
                return Ok(());
            }
            // One or more ';' and an optional trailing predicate.
            while self.peek() == Some(b';') {
                self.pos += 1;
                self.skip_ws();
            }
            if matches!(self.peek(), Some(b'.') | Some(b']') | None) {
                return Ok(());
            }
        }
    }

    fn verb(&mut self) -> Result<Term> {
        self.skip_ws();
        if self.peek() == Some(b'a')
            && self
                .peek_at(1)
                .is_none_or(|c| c.is_ascii_whitespace() || c == b'<' || c == b'[' || c == b'"')
        {
            self.pos += 1;
            return Ok(Term::iri(RDF_TYPE));
        }
        match self.peek() {
            Some(b'<') => Ok(Term::Iri(self.iriref()?)),
            _ => Ok(Term::Iri(self.prefixed_name()?)),
        }
    }

    fn object_list(&mut self, subject: &Term, predicate: &Term) -> Result<()> {
        loop {
            let object = self.object()?;
            self.emit(subject.clone(), predicate.clone(), object);
            self.skip_ws();
            if self.peek() == Some(b',') {
                self.pos += 1;
            } else {
                return Ok(());
            }
        }
    }

    fn object(&mut self) -> Result<Term> {
        self.skip_ws();
        match self.peek() {
            Some(b'<') => Ok(Term::Iri(self.iriref()?)),
            Some(b'_') if self.peek_at(1) == Some(b':') => self.blank_label(),
            Some(b'[') => self.blank_node_property_list(),
            Some(b'(') => self.collection(),
            Some(b'"') | Some(b'\'') => self.rdf_literal(),
            Some(c) if c.is_ascii_digit() || c == b'+' || c == b'-' || c == b'.' => {
                self.numeric_literal()
            }
            Some(_) if self.starts_with("true") && !self.name_char_at(4) => {
                self.pos += 4;
                Ok(Term::Literal(Literal::typed(
                    "true",
                    format!("{XSD}boolean"),
                )))
            }
            Some(_) if self.starts_with("false") && !self.name_char_at(5) => {
                self.pos += 5;
                Ok(Term::Literal(Literal::typed(
                    "false",
                    format!("{XSD}boolean"),
                )))
            }
            Some(_) => Ok(Term::Iri(self.prefixed_name()?)),
            None => self.err("unexpected end of input"),
        }
    }

    fn name_char_at(&self, off: usize) -> bool {
        self.peek_at(off)
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-' || c == b':')
    }

    fn blank_node_property_list(&mut self) -> Result<Term> {
        self.expect(b'[')?;
        let node = self.fresh_blank();
        self.skip_ws();
        if self.peek() != Some(b']') {
            self.predicate_object_list(&node)?;
        }
        self.expect(b']')?;
        Ok(node)
    }

    fn collection(&mut self) -> Result<Term> {
        self.expect(b'(')?;
        let mut items = Vec::new();
        loop {
            self.skip_ws();
            if self.peek() == Some(b')') {
                self.pos += 1;
                break;
            }
            items.push(self.object()?);
        }
        if items.is_empty() {
            return Ok(Term::iri(RDF_NIL));
        }
        let nodes: Vec<Term> = items.iter().map(|_| self.fresh_blank()).collect();
        for (i, item) in items.into_iter().enumerate() {
            self.emit(nodes[i].clone(), Term::iri(RDF_FIRST), item);
            let rest = nodes
                .get(i + 1)
                .cloned()
                .unwrap_or_else(|| Term::iri(RDF_NIL));
            self.emit(nodes[i].clone(), Term::iri(RDF_REST), rest);
        }
        Ok(nodes[0].clone())
    }

    fn blank_label(&mut self) -> Result<Term> {
        self.pos += 2; // "_:"
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_alphanumeric()
                || c == b'_'
                || c == b'-'
                || c >= 0x80
                || (c == b'.' && self.continues_name())
            {
                self.pos += 1;
            } else {
                break;
            }
        }
        if start == self.pos {
            return self.err("empty blank node label");
        }
        Ok(Term::BlankNode(format!(
            "l_{}",
            &self.text[start..self.pos]
        )))
    }

    /// A '.' is part of a name only if a name character follows it.
    fn continues_name(&self) -> bool {
        self.peek_at(1).is_some_and(|c| {
            c.is_ascii_alphanumeric()
                || c == b'_'
                || c == b'-'
                || c == b':'
                || c == b'%'
                || c >= 0x80
        })
    }

    fn iriref(&mut self) -> Result<String> {
        if self.peek() != Some(b'<') {
            return self.err("expected IRI");
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return self.err("unterminated IRI"),
                Some(b'>') => {
                    self.pos += 1;
                    break;
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = self.unicode_escape()?;
                    out.push(c);
                }
                Some(_) => {
                    let c = self.next_char();
                    if c.is_whitespace() {
                        return self.err("whitespace in IRI");
                    }
                    out.push(c);
                }
            }
        }
        Ok(match &self.base {
            Some(base) if !iri::is_absolute(&out) => iri::resolve(base, &out),
            _ => out,
        })
    }

    fn next_char(&mut self) -> char {
        let c = self.text[self.pos..].chars().next().expect("not at end");
        self.pos += c.len_utf8();
        c
    }

    fn unicode_escape(&mut self) -> Result<char> {
        let len = match self.peek() {
            Some(b'u') => 4,
            Some(b'U') => 8,
            _ => return self.err("bad escape"),
        };
        self.pos += 1;
        let end = self.pos + len;
        let hex = self
            .text
            .get(self.pos..end)
            .ok_or_else(|| Error::Syntax("short escape".into()))?;
        let code = u32::from_str_radix(hex, 16)
            .map_err(|_| Error::Syntax(format!("bad escape \\u{hex}")))?;
        self.pos = end;
        char::from_u32(code).ok_or_else(|| Error::Syntax(format!("bad code point {code:x}")))
    }

    fn prefixed_name(&mut self) -> Result<String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c == b':' {
                break;
            }
            if c.is_ascii_alphanumeric()
                || c == b'_'
                || c == b'-'
                || c >= 0x80
                || (c == b'.' && self.continues_name())
            {
                self.pos += 1;
            } else {
                return self.err("expected prefixed name");
            }
        }
        if self.peek() != Some(b':') {
            return self.err("expected prefixed name");
        }
        let prefix = self.text[start..self.pos].to_owned();
        self.pos += 1;
        let mut local = String::new();
        while let Some(c) = self.peek() {
            if c == b'\\' {
                // PN_LOCAL_ESC: the escaped character stands for itself.
                self.pos += 1;
                local.push(self.next_char());
            } else if c == b'%' {
                local.push_str(&self.text[self.pos..self.pos + 3]);
                self.pos += 3;
            } else if c.is_ascii_alphanumeric()
                || c == b'_'
                || c == b'-'
                || c == b':'
                || c >= 0x80
                || (c == b'.' && self.continues_name())
            {
                local.push(self.next_char());
            } else {
                break;
            }
        }
        match self.prefixes.get(&prefix) {
            Some(ns) => Ok(format!("{ns}{local}")),
            None => self.err(&format!("undefined prefix '{prefix}:'")),
        }
    }

    fn rdf_literal(&mut self) -> Result<Term> {
        let value = self.string()?;
        match self.peek() {
            Some(b'@') => {
                self.pos += 1;
                let start = self.pos;
                while let Some(c) = self.peek() {
                    if c.is_ascii_alphanumeric() || c == b'-' {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                Ok(Term::Literal(Literal::lang(
                    value,
                    &self.text[start..self.pos],
                )))
            }
            Some(b'^') if self.peek_at(1) == Some(b'^') => {
                self.pos += 2;
                let dt = if self.peek() == Some(b'<') {
                    self.iriref()?
                } else {
                    self.prefixed_name()?
                };
                Ok(Term::Literal(Literal::typed(value, dt)))
            }
            _ => Ok(Term::Literal(Literal::simple(value))),
        }
    }

    fn string(&mut self) -> Result<String> {
        let quote = self.peek().expect("at quote");
        let long = self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote);
        self.pos += if long { 3 } else { 1 };
        let mut out = String::new();
        loop {
            match self.peek() {
                None => return self.err("unterminated string"),
                Some(c) if c == quote => {
                    if !long {
                        self.pos += 1;
                        return Ok(out);
                    }
                    if self.peek_at(1) == Some(quote) && self.peek_at(2) == Some(quote) {
                        // """ ends the string, but a quote right before it belongs to the value.
                        if self.peek_at(3) == Some(quote) {
                            out.push(quote as char);
                            self.pos += 1;
                            continue;
                        }
                        self.pos += 3;
                        return Ok(out);
                    }
                    out.push(quote as char);
                    self.pos += 1;
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let c = match self.peek() {
                        Some(b't') => '\t',
                        Some(b'b') => '\u{8}',
                        Some(b'n') => '\n',
                        Some(b'r') => '\r',
                        Some(b'f') => '\u{c}',
                        Some(b'"') => '"',
                        Some(b'\'') => '\'',
                        Some(b'\\') => '\\',
                        Some(b'u') | Some(b'U') => {
                            let c = self.unicode_escape()?;
                            out.push(c);
                            continue;
                        }
                        _ => return self.err("bad string escape"),
                    };
                    self.pos += 1;
                    out.push(c);
                }
                Some(b'\n') | Some(b'\r') if !long => return self.err("newline in string"),
                Some(_) => out.push(self.next_char()),
            }
        }
    }

    fn numeric_literal(&mut self) -> Result<Term> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.pos += 1;
        }
        let mut seen_dot = false;
        let mut seen_exp = false;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() {
                self.pos += 1;
            } else if c == b'.'
                && !seen_dot
                && !seen_exp
                && self.peek_at(1).is_some_and(|d| d.is_ascii_digit())
            {
                seen_dot = true;
                self.pos += 1;
            } else if (c == b'e' || c == b'E') && !seen_exp {
                seen_exp = true;
                self.pos += 1;
                if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
        let lexical = &self.text[start..self.pos];
        if lexical.is_empty() || lexical == "+" || lexical == "-" {
            return self.err("bad number");
        }
        let dt = if seen_exp {
            "double"
        } else if seen_dot {
            "decimal"
        } else {
            "integer"
        };
        Ok(Term::Literal(Literal::typed(lexical, format!("{XSD}{dt}"))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rml_mapping() {
        let ttl = r#"
            @prefix rml: <http://w3id.org/rml/> .
            @prefix ex: <http://example.com/> .
            <http://example.com/base/TM> a rml:TriplesMap;
              rml:logicalSource [ rml:iterator "$.students[*]" ];
              rml:subjectMap [ rml:template "http://example.com/{$.ID}" ; rml:class ex:P, ex:Q ] .
        "#;
        let triples = parse(ttl, None).unwrap();
        assert_eq!(triples.len(), 7);
        assert!(
            triples
                .iter()
                .any(|t| t.object == Term::Literal(Literal::simple("$.students[*]")))
        );
    }

    #[test]
    fn long_strings_and_numbers() {
        let ttl = r#"@prefix ex: <http://e/> . ex:a ex:b """x "q" y""", 'z', 1, -2.5, 3e1, true ."#;
        let t = parse(ttl, None).unwrap();
        let objs: Vec<String> = t.iter().map(|t| t.object.to_string()).collect();
        assert_eq!(objs[0], r#""x \"q\" y""#);
        assert!(objs[2].contains("integer"));
        assert!(objs[3].contains("decimal"));
        assert!(objs[4].contains("double"));
        assert!(objs[5].contains("boolean"));
    }

    #[test]
    fn template_escapes_survive() {
        let ttl = r#"@prefix ex: <http://e/> . ex:a ex:b "\\{\\{ {Title} \\}" ."#;
        let t = parse(ttl, None).unwrap();
        assert_eq!(
            t[0].object,
            Term::Literal(Literal::simple(r"\{\{ {Title} \}"))
        );
    }

    #[test]
    fn relative_iri_and_collection() {
        let ttl = "@base <http://e/x/> . <a> <b> ( <c> <d> ) .";
        let t = parse(ttl, None).unwrap();
        assert_eq!(
            t.iter()
                .filter(|t| t.subject == Term::iri("http://e/x/a"))
                .count(),
            1
        );
        assert_eq!(t.len(), 5);
    }
}
