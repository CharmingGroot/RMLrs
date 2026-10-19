// SPDX-License-Identifier: Apache-2.0
//! IRI checks and resolution (RFC 3986 / RFC 3987).

/// `scheme ":" ...` with a valid scheme.
pub fn is_absolute(s: &str) -> bool {
    scheme_len(s).is_some()
}

fn scheme_len(s: &str) -> Option<usize> {
    let colon = s.find(':')?;
    let scheme = &s[..colon];
    let mut chars = scheme.chars();
    let first = chars.next()?;
    if !first.is_ascii_alphabetic() {
        return None;
    }
    if chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')) {
        Some(colon)
    } else {
        None
    }
}

/// A valid absolute IRI per RFC 3987: a scheme, and no character that may not
/// appear anywhere in an IRI. Percent signs must start a `%XX` escape.
pub fn is_valid_absolute_iri(s: &str) -> bool {
    if !is_absolute(s) {
        return false;
    }
    let bytes = s.as_bytes();
    let mut i = 0;
    for c in s.chars() {
        let len = c.len_utf8();
        match c {
            '%' => {
                let ok = bytes.get(i + 1).is_some_and(u8::is_ascii_hexdigit)
                    && bytes.get(i + 2).is_some_and(u8::is_ascii_hexdigit);
                if !ok {
                    return false;
                }
            }
            c if is_iri_char(c) => {}
            _ => return false,
        }
        i += len;
    }
    true
}

/// Characters that may appear in some part of an IRI (unreserved, reserved,
/// `%`, ucschar and iprivate).
fn is_iri_char(c: char) -> bool {
    match c {
        'a'..='z' | 'A'..='Z' | '0'..='9' => true,
        '-' | '.' | '_' | '~' => true,
        ':' | '/' | '?' | '#' | '[' | ']' | '@' => true,
        '!' | '$' | '&' | '\'' | '(' | ')' | '*' | '+' | ',' | ';' | '=' => true,
        '%' => true,
        c => is_ucschar(c) || is_iprivate(c),
    }
}

/// `iunreserved` minus the ASCII part: characters allowed unescaped in IRIs.
pub fn is_ucschar(c: char) -> bool {
    matches!(c as u32,
        0xA0..=0xD7FF | 0xF900..=0xFDCF | 0xFDF0..=0xFFEF
        | 0x10000..=0x1FFFD | 0x20000..=0x2FFFD | 0x30000..=0x3FFFD
        | 0x40000..=0x4FFFD | 0x50000..=0x5FFFD | 0x60000..=0x6FFFD
        | 0x70000..=0x7FFFD | 0x80000..=0x8FFFD | 0x90000..=0x9FFFD
        | 0xA0000..=0xAFFFD | 0xB0000..=0xBFFFD | 0xC0000..=0xCFFFD
        | 0xD0000..=0xDFFFD | 0xE1000..=0xEFFFD)
}

fn is_iprivate(c: char) -> bool {
    matches!(c as u32, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD)
}

/// Resolves `reference` against `base` (RFC 3986 section 5.2).
pub fn resolve(base: &str, reference: &str) -> String {
    if is_absolute(reference) {
        return reference.to_owned();
    }
    let b = split(base);
    let r = split(reference);
    let (authority, path, query);
    if r.authority.is_some() {
        authority = r.authority;
        path = remove_dot_segments(r.path);
        query = r.query;
    } else {
        authority = b.authority;
        if r.path.is_empty() {
            path = b.path.to_owned();
            query = r.query.or(b.query);
        } else {
            path = if r.path.starts_with('/') {
                remove_dot_segments(r.path)
            } else {
                remove_dot_segments(&merge(&b, r.path))
            };
            query = r.query;
        }
    }
    let mut out = String::new();
    if let Some(s) = b.scheme {
        out.push_str(s);
        out.push(':');
    }
    if let Some(a) = authority {
        out.push_str("//");
        out.push_str(a);
    }
    out.push_str(&path);
    if let Some(q) = query {
        out.push('?');
        out.push_str(q);
    }
    if let Some(f) = r.fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

struct Parts<'a> {
    scheme: Option<&'a str>,
    authority: Option<&'a str>,
    path: &'a str,
    query: Option<&'a str>,
    fragment: Option<&'a str>,
}

fn split(s: &str) -> Parts<'_> {
    let (s, fragment) = match s.find('#') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (s, query) = match s.find('?') {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let (scheme, rest) = match scheme_len(s) {
        Some(i) => (Some(&s[..i]), &s[i + 1..]),
        None => (None, s),
    };
    let (authority, path) = match rest.strip_prefix("//") {
        Some(r) => {
            let end = r.find('/').unwrap_or(r.len());
            (Some(&r[..end]), &r[end..])
        }
        None => (None, rest),
    };
    Parts {
        scheme,
        authority,
        path,
        query,
        fragment,
    }
}

fn merge(base: &Parts<'_>, path: &str) -> String {
    if base.authority.is_some() && base.path.is_empty() {
        return format!("/{path}");
    }
    match base.path.rfind('/') {
        Some(i) => format!("{}{}", &base.path[..=i], path),
        None => path.to_owned(),
    }
}

fn remove_dot_segments(path: &str) -> String {
    let mut input = path.to_owned();
    let mut output: Vec<String> = Vec::new();
    while !input.is_empty() {
        if let Some(rest) = input.strip_prefix("../") {
            input = rest.to_owned();
        } else if let Some(rest) = input.strip_prefix("./") {
            input = rest.to_owned();
        } else if input.starts_with("/./") {
            input = input[2..].to_owned();
        } else if input == "/." {
            input = "/".to_owned();
        } else if input.starts_with("/../") {
            input = input[3..].to_owned();
            output.pop();
        } else if input == "/.." {
            input = "/".to_owned();
            output.pop();
        } else if input == "." || input == ".." {
            input.clear();
        } else {
            let start = usize::from(input.starts_with('/'));
            let end = input[start..].find('/').map_or(input.len(), |i| i + start);
            output.push(input[..end].to_owned());
            input = input[end..].to_owned();
        }
    }
    output.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3986_examples() {
        let base = "http://a/b/c/d;p?q";
        for (r, want) in [
            ("g", "http://a/b/c/g"),
            ("./g", "http://a/b/c/g"),
            ("g/", "http://a/b/c/g/"),
            ("/g", "http://a/g"),
            ("//g", "http://g"),
            ("?y", "http://a/b/c/d;p?y"),
            ("#s", "http://a/b/c/d;p?q#s"),
            ("", "http://a/b/c/d;p?q"),
            ("..", "http://a/b/"),
            ("../g", "http://a/b/g"),
            ("../../g", "http://a/g"),
            ("../../../g", "http://a/g"),
        ] {
            assert_eq!(resolve(base, r), want, "{r}");
        }
    }

    #[test]
    fn validity() {
        assert!(is_valid_absolute_iri("http://example.com/10/Venus"));
        assert!(is_valid_absolute_iri("http://example.com/Zoë"));
        assert!(!is_valid_absolute_iri("http://example.com/Juan Daniel"));
        assert!(!is_valid_absolute_iri("Venus"));
        assert!(!is_valid_absolute_iri("http://e/%zz"));
    }
}
