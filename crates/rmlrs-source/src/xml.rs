// SPDX-License-Identifier: Apache-2.0
//! XML sources with XPath (RML-IO-Registry, `rml:XPath`).
//!
//! The iterator and the references are XPath 3.1 expressions, evaluated by
//! the `xee-xpath` crate. The iterator is evaluated against the document
//! node (`/` when there is none) and must select nodes; each node is one
//! iteration and the context of the references. A reference gives the
//! string value of every item it selects, in document order. Nothing
//! selected means NULL. XML values carry no data types, so every value is an
//! `xsd:string`.
//!
//! References can walk up from the iteration (`../@id`), so in general the
//! whole document is parsed into a tree and kept while its iterations are in
//! use. When the iterator is a plain path of child steps (`/a/b/c`) and no
//! reference looks outside the iterated element ([`is_local`]), the
//! document is instead cut into the selected elements and parsed a chunk of
//! elements at a time ([`local_iterations`]): the tree then never holds more
//! than one chunk.

use std::io::BufRead;

use rmlrs_model::{Error, Result};

use crate::NaturalValue;

/// Prefix and namespace IRI pairs declared by the reference formulation.
pub type Namespaces = [(String, String)];

pub use imp::{Node, XPath};

/// Compiles an XPath expression with the given namespace prefixes.
pub fn compile(expression: &str, namespaces: &Namespaces) -> Result<XPath> {
    imp::compile(expression, namespaces)
}

/// Parses a document and returns its iterations: the nodes the iterator
/// selects. `set` tags the nodes with the namespace set they were made for.
pub fn iterations(
    reader: Box<dyn BufRead + '_>,
    iterator: Option<&str>,
    namespaces: &Namespaces,
    set: u32,
) -> Result<Vec<Node>> {
    imp::iterations(reader, iterator, namespaces, set)
}

/// Like [`iterations`], for a logical source whose references all pass
/// [`is_local`]: the selected elements are parsed a chunk at a time. Falls
/// back to the whole-document tree when the iterator is not a plain path.
pub fn local_iterations<'a>(
    reader: Box<dyn BufRead + 'a>,
    iterator: Option<&str>,
    namespaces: &Namespaces,
    set: u32,
) -> Result<Box<dyn Iterator<Item = Result<Node>> + 'a>> {
    imp::local_iterations(reader, iterator, namespaces, set)
}

/// Whether a reference only looks at the iterated element and what is
/// inside it. Conservative: anything that might reach a parent, a sibling or
/// the document (`..`, axes, `root()`, absolute paths, ...) is not local.
pub fn is_local(reference: &str) -> bool {
    const OUTSIDE: [&str; 12] = [
        "..",
        "ancestor",
        "parent",
        "preceding",
        "following",
        "root",
        "id(",
        "idref",
        "doc",
        "collection",
        "base-uri",
        "lang(",
    ];
    !reference.trim_start().starts_with('/') && !OUTSIDE.iter().any(|w| reference.contains(w))
}

/// Evaluates a compiled reference against one iteration.
pub fn evaluate(node: &Node, reference: &XPath) -> Result<Vec<NaturalValue>> {
    imp::evaluate(node, reference)
}

#[cfg(not(feature = "xml"))]
mod imp {
    use super::*;

    fn unsupported<T>() -> Result<T> {
        Err(Error::Unsupported(
            "rml:XPath (build with the \"xml\" feature)".into(),
        ))
    }

    /// Uninhabited without the `xml` feature.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum XPath {}

    /// Uninhabited without the `xml` feature.
    #[derive(Debug, Clone, PartialEq)]
    pub enum Node {}

    impl Node {
        pub fn namespace_set(&self) -> u32 {
            match *self {}
        }
    }

    pub fn compile(_: &str, _: &Namespaces) -> Result<XPath> {
        unsupported()
    }

    pub fn iterations(
        _: Box<dyn BufRead + '_>,
        _: Option<&str>,
        _: &Namespaces,
        _: u32,
    ) -> Result<Vec<Node>> {
        unsupported()
    }

    pub fn local_iterations<'a>(
        _: Box<dyn BufRead + 'a>,
        _: Option<&str>,
        _: &Namespaces,
        _: u32,
    ) -> Result<Box<dyn Iterator<Item = Result<Node>> + 'a>> {
        unsupported()
    }

    pub fn evaluate(node: &Node, _: &XPath) -> Result<Vec<NaturalValue>> {
        match *node {}
    }
}

#[cfg(feature = "xml")]
mod imp {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;
    use xee_xpath::context::StaticContextBuilder;
    use xee_xpath::query::SequenceQuery;
    use xee_xpath::{Documents, Item, Queries, Query};

    /// A compiled XPath expression. Two are equal when their text is.
    #[derive(Clone)]
    pub struct XPath {
        text: String,
        query: SequenceQuery,
    }

    impl std::fmt::Debug for XPath {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "XPath({:?})", self.text)
        }
    }

    impl PartialEq for XPath {
        fn eq(&self, other: &Self) -> bool {
            self.text == other.text
        }
    }

    impl Eq for XPath {}

    /// One iteration: a node of a parsed document.
    #[derive(Clone)]
    pub struct Node {
        documents: Rc<RefCell<Documents>>,
        node: xot::Node,
        set: u32,
    }

    impl Node {
        /// The namespace set the node's document was opened with.
        pub fn namespace_set(&self) -> u32 {
            self.set
        }
    }

    impl std::fmt::Debug for Node {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "Node({:?})", self.node)
        }
    }

    impl PartialEq for Node {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.documents, &other.documents) && self.node == other.node
        }
    }

    pub fn compile(expression: &str, namespaces: &Namespaces) -> Result<XPath> {
        let mut context = StaticContextBuilder::default();
        for (prefix, iri) in namespaces {
            context.add_namespace(prefix, iri);
        }
        let query = Queries::new(context)
            .sequence(expression)
            .map_err(|e| Error::Mapping(format!("invalid XPath {expression:?}: {e}")))?;
        Ok(XPath {
            text: expression.to_owned(),
            query,
        })
    }

    fn read(mut reader: Box<dyn BufRead + '_>) -> Result<String> {
        let mut text = String::new();
        reader
            .read_to_string(&mut text)
            .map_err(|e| Error::Source(e.to_string()))?;
        Ok(text)
    }

    pub fn iterations(
        reader: Box<dyn BufRead + '_>,
        iterator: Option<&str>,
        namespaces: &Namespaces,
        set: u32,
    ) -> Result<Vec<Node>> {
        let iterator = compile(iterator.unwrap_or("/"), namespaces)?;
        tree_iterations(read(reader)?, &iterator, set)
    }

    fn tree_iterations(text: String, iterator: &XPath, set: u32) -> Result<Vec<Node>> {
        let mut documents = Documents::new();
        let doc = documents
            .add_string_without_uri(&text)
            .map_err(|e| Error::Source(format!("invalid XML: {e}")))?;
        drop(text);
        let selected = iterator
            .query
            .execute(&mut documents, doc)
            .map_err(|e| Error::Data(format!("XPath iterator {:?}: {e}", iterator.text)))?;
        let documents = Rc::new(RefCell::new(documents));
        selected
            .iter()
            .map(|item| match item {
                Item::Node(node) => Ok(Node {
                    documents: Rc::clone(&documents),
                    node,
                    set,
                }),
                _ => Err(Error::Data(format!(
                    "XPath iterator {:?} selects a value that is not a node",
                    iterator.text
                ))),
            })
            .collect()
    }

    pub fn local_iterations<'a>(
        reader: Box<dyn BufRead + 'a>,
        iterator: Option<&str>,
        namespaces: &Namespaces,
        set: u32,
    ) -> Result<Box<dyn Iterator<Item = Result<Node>> + 'a>> {
        let compiled = compile(iterator.unwrap_or("/"), namespaces)?;
        let text = read(reader)?;
        let fragments = iterator
            .and_then(|it| simple_steps(it, namespaces))
            .and_then(|steps| split(&text, &steps));
        match fragments {
            Some(fragments) => Ok(Box::new(Chunks {
                text,
                fragments: fragments.into_iter(),
                children: compile("/rmlrs-chunk/*", &[])?,
                set,
                current: Vec::new().into_iter(),
            })),
            None => Ok(Box::new(
                tree_iterations(text, &compiled, set)?.into_iter().map(Ok),
            )),
        }
    }

    /// The steps of an iterator of the form `/a/b/p:c`, as (namespace IRI,
    /// local name); `None` for anything else.
    fn simple_steps(iterator: &str, namespaces: &Namespaces) -> Option<Vec<(String, String)>> {
        let name = |s: &str| {
            s.chars()
                .next()
                .is_some_and(|c| c.is_alphabetic() || c == '_')
                && s.chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.'))
        };
        iterator
            .strip_prefix('/')?
            .split('/')
            .map(|step| {
                let (prefix, local) = match step.split_once(':') {
                    Some((p, l)) => (Some(p), l),
                    None => (None, step),
                };
                if !name(local) || prefix.is_some_and(|p| !name(p)) {
                    return None;
                }
                let ns = match prefix {
                    None => String::new(),
                    Some(p) => namespaces.iter().find(|(q, _)| q == p)?.1.clone(),
                };
                Some((ns, local.to_owned()))
            })
            .collect()
    }

    /// A selected element: its text is `start..end`; `decls` are the
    /// namespace declarations it inherits, inserted after its name
    /// (`name_end`) so that it parses on its own.
    struct Fragment {
        start: usize,
        name_end: usize,
        end: usize,
        decls: Rc<str>,
    }

    /// A start tag being read: name, where it starts and where its name
    /// ends, and its namespace declarations (prefix, raw value).
    struct StartTag<'a> {
        prefix: &'a str,
        local: &'a str,
        start: usize,
        name_end: usize,
        decls: Vec<(String, String)>,
    }

    struct Open {
        /// Namespace declarations on this element: (prefix, raw value).
        decls: Vec<(String, String)>,
        /// The element and its ancestors match the first steps.
        matched: bool,
        fragment: Option<Fragment>,
    }

    /// Cuts a document into the elements the steps select. `None` when the
    /// document has a DTD (entities could change the text) or anything this
    /// simple reading cannot be sure about; the caller then builds the tree.
    fn split(text: &str, steps: &[(String, String)]) -> Option<Vec<Fragment>> {
        use xmlparser::{ElementEnd, Token, Tokenizer};
        const XML_NS: &str = "http://www.w3.org/XML/1998/namespace";
        let mut stack: Vec<Open> = Vec::new();
        let mut out = Vec::new();
        let mut pending: Option<StartTag> = None;
        let mut last_decls: Rc<str> = Rc::from("");
        for token in Tokenizer::from(text) {
            match token.ok()? {
                Token::DtdStart { .. } | Token::EmptyDtd { .. } => return None,
                Token::ElementStart {
                    prefix,
                    local,
                    span,
                } => {
                    pending = Some(StartTag {
                        prefix: prefix.as_str(),
                        local: local.as_str(),
                        start: span.start(),
                        name_end: local.end(),
                        decls: Vec::new(),
                    });
                }
                Token::Attribute {
                    prefix,
                    local,
                    value,
                    ..
                } => {
                    let decls = &mut pending.as_mut()?.decls;
                    let v = value.as_str();
                    if v.contains(['&', '"']) {
                        return None;
                    }
                    if prefix.as_str() == "xmlns" {
                        decls.push((local.as_str().to_owned(), v.to_owned()));
                    } else if prefix.is_empty() && local.as_str() == "xmlns" {
                        decls.push((String::new(), v.to_owned()));
                    }
                }
                Token::ElementEnd {
                    end: end @ (ElementEnd::Open | ElementEnd::Empty),
                    span,
                } => {
                    let StartTag {
                        prefix,
                        local,
                        start,
                        name_end,
                        decls,
                    } = pending.take()?;
                    let depth = stack.len();
                    let lookup = |p: &str| -> Option<String> {
                        if p == "xml" {
                            return Some(XML_NS.to_owned());
                        }
                        decls
                            .iter()
                            .chain(stack.iter().rev().flat_map(|o| o.decls.iter()))
                            .find(|(q, _)| q == p)
                            .map(|(_, v)| v.clone())
                    };
                    let ns = match lookup(prefix) {
                        Some(v) => v,
                        None if prefix.is_empty() => String::new(),
                        None => return None,
                    };
                    let matched = (depth == 0 || stack[depth - 1].matched)
                        && steps
                            .get(depth)
                            .is_some_and(|(sn, sl)| *sn == ns && sl == local);
                    let fragment = if matched && depth + 1 == steps.len() {
                        let mut inherited = String::new();
                        let mut done: Vec<&str> = decls.iter().map(|(p, _)| p.as_str()).collect();
                        for (p, v) in stack.iter().rev().flat_map(|o| o.decls.iter()) {
                            if done.contains(&p.as_str()) {
                                continue;
                            }
                            done.push(p);
                            if p.is_empty() {
                                inherited.push_str(&format!(" xmlns=\"{v}\""));
                            } else {
                                inherited.push_str(&format!(" xmlns:{p}=\"{v}\""));
                            }
                        }
                        if *last_decls != *inherited {
                            last_decls = Rc::from(inherited);
                        }
                        Some(Fragment {
                            start,
                            name_end,
                            end: span.end(),
                            decls: Rc::clone(&last_decls),
                        })
                    } else {
                        None
                    };
                    match end {
                        ElementEnd::Empty => out.extend(fragment),
                        _ => stack.push(Open {
                            decls,
                            matched,
                            fragment,
                        }),
                    }
                }
                Token::ElementEnd {
                    end: ElementEnd::Close(..),
                    span,
                } => {
                    let open = stack.pop()?;
                    if let Some(mut f) = open.fragment {
                        f.end = span.end();
                        out.push(f);
                    }
                }
                _ => {}
            }
        }
        Some(out)
    }

    /// Text of selected elements parsed per chunk: at most this many bytes
    /// of element text go into one tree.
    const CHUNK_BYTES: usize = 256 * 1024;

    struct Chunks {
        text: String,
        fragments: std::vec::IntoIter<Fragment>,
        children: XPath,
        set: u32,
        current: std::vec::IntoIter<Node>,
    }

    impl Chunks {
        fn next_chunk(&mut self) -> Result<bool> {
            let mut chunk = String::from("<rmlrs-chunk>");
            let mut any = false;
            while chunk.len() < CHUNK_BYTES {
                let Some(f) = self.fragments.next() else {
                    break;
                };
                chunk.push_str(&self.text[f.start..f.name_end]);
                chunk.push_str(&f.decls);
                chunk.push_str(&self.text[f.name_end..f.end]);
                any = true;
            }
            if !any {
                return Ok(false);
            }
            chunk.push_str("</rmlrs-chunk>");
            self.current = tree_iterations(chunk, &self.children, self.set)?.into_iter();
            Ok(true)
        }
    }

    impl Iterator for Chunks {
        type Item = Result<Node>;

        fn next(&mut self) -> Option<Result<Node>> {
            loop {
                if let Some(n) = self.current.next() {
                    return Some(Ok(n));
                }
                match self.next_chunk() {
                    Ok(true) => {}
                    Ok(false) => return None,
                    Err(e) => return Some(Err(e)),
                }
            }
        }
    }

    pub fn evaluate(node: &Node, reference: &XPath) -> Result<Vec<NaturalValue>> {
        let mut documents = node.documents.borrow_mut();
        let selected = reference
            .query
            .execute(&mut documents, node.node)
            .map_err(|e| Error::Data(format!("XPath {:?}: {e}", reference.text)))?;
        selected
            .iter()
            .map(|item| {
                item.string_value(documents.xot())
                    .map(NaturalValue::string)
                    .map_err(|e| Error::Data(format!("XPath {:?}: {e}", reference.text)))
            })
            .collect()
    }
}

#[cfg(all(test, feature = "xml"))]
mod tests {
    use super::*;

    const DOC: &str = r#"<?xml version="1.0"?>
<companies xmlns:ex="http://example.org/">
  <company id="25"><name>TechCorp</name>
    <departments><department><name>Engineering</name></department>
                 <department><name>Marketing</name></department></departments>
  </company>
  <ex:company id="35"><ex:name>Other</ex:name></ex:company>
</companies>"#;

    fn values(iterator: &str, reference: &str, ns: &Namespaces) -> Vec<Vec<String>> {
        let p = compile(reference, ns).unwrap();
        iterations(Box::new(DOC.as_bytes()), Some(iterator), ns, 0)
            .unwrap()
            .iter()
            .map(|n| {
                evaluate(n, &p)
                    .unwrap()
                    .into_iter()
                    .map(|v| v.lexical)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn iterations_and_references() {
        assert_eq!(values("/companies/company", "@id", &[]), [["25"]]);
        // Several values, in document order; walking up to the parent.
        assert_eq!(
            values("//departments", "department/name", &[]),
            [["Engineering", "Marketing"]]
        );
        assert_eq!(values("//department", "../../@id", &[]), [["25"], ["25"]]);
        // Nothing selected is NULL, not an error.
        assert_eq!(values("/companies/company", "missing", &[]), [[""; 0]]);
    }

    #[test]
    fn namespaces() {
        let ns = [("e".to_owned(), "http://example.org/".to_owned())];
        assert_eq!(values("/companies/e:company", "e:name", &ns), [["Other"]]);
        // An undeclared prefix is a mapping error.
        assert!(matches!(compile("x:name", &[]), Err(Error::Mapping(_))));
    }

    /// Values of `reference` per iteration, read a chunk at a time.
    fn local_values(
        doc: &str,
        iterator: &str,
        reference: &str,
        ns: &Namespaces,
    ) -> Vec<Vec<String>> {
        let p = compile(reference, ns).unwrap();
        local_iterations(Box::new(doc.as_bytes()), Some(iterator), ns, 0)
            .unwrap()
            .map(|n| {
                evaluate(&n.unwrap(), &p)
                    .unwrap()
                    .into_iter()
                    .map(|v| v.lexical)
                    .collect()
            })
            .collect()
    }

    #[test]
    fn chunks_give_the_same_values_as_the_tree() {
        let ns = [("e".to_owned(), "http://example.org/".to_owned())];
        for (it, r, n) in [
            ("/companies/company", "@id", &[][..]),
            ("/companies/company", "departments/department/name", &[]),
            ("/companies/e:company", "e:name", &ns),
            ("/companies/company", "missing", &[]),
        ] {
            let chunked = local_values(DOC, it, r, n);
            assert_eq!(chunked, values(it, r, n), "{it} {r}");
        }
        // Inherited default and prefixed namespaces travel with the element.
        let doc = r#"<r xmlns="http://d/" xmlns:x="http://x/"><a x:k="1"><x:b>v</x:b></a><a x:k="2"/></r>"#;
        let ns = [
            ("d".to_owned(), "http://d/".to_owned()),
            ("x".to_owned(), "http://x/".to_owned()),
        ];
        assert_eq!(local_values(doc, "/d:r/d:a", "@x:k", &ns), [["1"], ["2"]]);
        assert_eq!(
            local_values(doc, "/d:r/d:a", "x:b", &ns),
            [vec!["v"], vec![]]
        );
        // An unprefixed step means no namespace, so nothing matches here.
        assert!(local_values(doc, "/r/a", "@x:k", &ns).is_empty());
        // Not a plain path: the whole tree is used, with the same result.
        assert_eq!(
            local_values(DOC, "//department", "name", &[]),
            [["Engineering"], ["Marketing"]]
        );
    }

    #[test]
    fn local_references() {
        for r in ["@id", "name", "a/b/text()", "concat(@a, '-', b)"] {
            assert!(is_local(r), "{r}");
        }
        for r in [
            "../@id",
            "/a/b",
            "//b",
            "ancestor::a/@id",
            "preceding-sibling::x",
            "root()",
        ] {
            assert!(!is_local(r), "{r}");
        }
    }

    #[test]
    fn errors() {
        assert!(matches!(compile("???", &[]), Err(Error::Mapping(_))));
        assert!(matches!(
            iterations(Box::new("<a>".as_bytes()), None, &[], 0),
            Err(Error::Source(_))
        ));
        assert!(matches!(
            iterations(Box::new("<a/>".as_bytes()), Some("count(/a)"), &[], 0),
            Err(Error::Data(_))
        ));
    }
}
