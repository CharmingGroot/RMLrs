// SPDX-License-Identifier: Apache-2.0
//! Reads an RML mapping graph into the [`Mapping`] model.
//!
//! Shortcuts (`rml:subject`, `rml:predicate`, `rml:object`, `rml:graph`,
//! `rml:datatype`, `rml:language`, `rml:child`, `rml:parent`) are expanded
//! here, default term types are resolved, and structural rules from RML-Core
//! are checked. The engine can then trust the model.

use std::collections::HashMap;

use rmlrs_model::{
    Collection, Compression, CsvDialect, CsvwTable, DatabaseSource, Error, Expression, Field,
    FieldKind, FunctionCall, GatherMap, JoinCondition, LogicalSource, LogicalTarget, LogicalView,
    Mapping, ObjectMap, PathRoot, PredicateObjectMap, RDF, RDF_FIRST, RDF_NIL, RDF_REST, RDF_TYPE,
    RefObjectMap, ReferenceFormulation, RemoteSource, Result, Serialization, Source,
    SourceEncoding, SparqlResultsFormat, Strategy, SubjectMap, Term, TermMap, TermType,
    TextEncoding, Trim, Triple, TriplesMap, ViewJoin,
};

use rmlrs_term::is_valid_language_tag;

use crate::template;

pub const RML: &str = "http://w3id.org/rml/";

/// The `rml:` terms this parser understands.
const IMPLEMENTED: &[&str] = &[
    // RML-Core classes
    "TriplesMap",
    "LogicalSource",
    "SubjectMap",
    "PredicateObjectMap",
    "PredicateMap",
    "ObjectMap",
    "GraphMap",
    "RefObjectMap",
    "ReferencingObjectMap",
    "Join",
    "JoinCondition",
    "TermMap",
    "ExpressionMap",
    "DatatypeMap",
    "LanguageMap",
    "ChildMap",
    "ParentMap",
    // RML-Core properties
    "logicalSource",
    "subjectMap",
    "subject",
    "predicateObjectMap",
    "predicateMap",
    "predicate",
    "objectMap",
    "object",
    "graphMap",
    "graph",
    "constant",
    "reference",
    "template",
    "termType",
    "datatype",
    "datatypeMap",
    "language",
    "languageMap",
    "class",
    "parentTriplesMap",
    "joinCondition",
    "child",
    "parent",
    "childMap",
    "parentMap",
    "baseIRI",
    "iterator",
    "referenceFormulation",
    // RML-Core individuals
    "IRI",
    "UnsafeIRI",
    "URI",
    "UnsafeURI",
    "BlankNode",
    "Literal",
    "defaultGraph",
    // RML-IO: local files and JSONPath only
    "source",
    "Source",
    "FilePath",
    "RelativePathSource",
    "path",
    "root",
    "MappingDirectory",
    "CurrentWorkingDirectory",
    "JSONPath",
    "CSV",
    "XPath",
    "XPathReferenceFormulation",
    "namespace",
    "Namespace",
    "namespacePrefix",
    "namespaceURL",
    "SQL2008Table",
    "SQL2008Query",
    "SQLTable",
    "SQLQuery",
    "encoding",
    "UTF-8",
    "null",
    // RML-IO compression of file sources and targets
    "compression",
    "none",
    "gzip",
    "zip",
    "targz",
    "tarxz",
    // RML-IO logical targets (file targets)
    "logicalTarget",
    "LogicalTarget",
    "target",
    "Target",
    "serialization",
    "UTF-16",
    // RML-FNML
    "functionExecution",
    "FunctionExecution",
    "Execution",
    "function",
    "functionMap",
    "FunctionMap",
    "input",
    "Input",
    "parameter",
    "parameterMap",
    "ParameterMap",
    "inputValue",
    "inputValueMap",
    "return",
    "returnMap",
    "ReturnMap",
    "condition",
    // RML-CC
    "GatherMap",
    "Strategy",
    "gather",
    "gatherAs",
    "strategy",
    "append",
    "cartesianProduct",
    "allowEmptyListAndContainer",
    // RML-LV
    "LogicalView",
    "AbstractLogicalSource",
    "viewOn",
    "field",
    "Field",
    "ExpressionField",
    "IterableField",
    "fieldName",
    "leftJoin",
    "innerJoin",
    "LogicalViewJoin",
    "parentLogicalView",
    // RML-star
    "AssertedTriplesMap",
    "NonAssertedTriplesMap",
    "StarMap",
    "quotedTriplesMap",
];

fn rml(local: &str) -> Term {
    Term::Iri(format!("{RML}{local}"))
}

/// The mapping graph, indexed by subject.
struct Graph {
    by_subject: HashMap<Term, Vec<(Term, Term)>>,
    order: Vec<Term>,
    /// Logical target nodes and the index of their (deduplicated) target.
    target_index: HashMap<Term, usize>,
}

impl Graph {
    fn new(triples: Vec<Triple>) -> Self {
        let mut by_subject: HashMap<Term, Vec<(Term, Term)>> = HashMap::new();
        let mut order = Vec::new();
        for t in triples {
            let entry = by_subject.entry(t.subject.clone()).or_insert_with(|| {
                order.push(t.subject.clone());
                Vec::new()
            });
            entry.push((t.predicate, t.object));
        }
        Graph {
            by_subject,
            order,
            target_index: HashMap::new(),
        }
    }

    fn objects<'a>(&'a self, s: &Term, p: &'a Term) -> impl Iterator<Item = &'a Term> + 'a {
        self.by_subject
            .get(s)
            .into_iter()
            .flatten()
            .filter(move |(pp, _)| pp == p)
            .map(|(_, o)| o)
    }

    fn all(&self, s: &Term, p: &str) -> Vec<Term> {
        let p = rml(p);
        self.objects(s, &p).cloned().collect()
    }

    fn one(&self, s: &Term, p: &str) -> Result<Option<Term>> {
        let mut v = self.all(s, p);
        match v.len() {
            0 => Ok(None),
            1 => Ok(v.pop()),
            _ => Err(Error::Mapping(format!("{s} has more than one rml:{p}"))),
        }
    }

    /// The members of an RDF list, in order.
    fn list(&self, head: &Term) -> Result<Vec<Term>> {
        let (first, rest) = (Term::iri(RDF_FIRST), Term::iri(RDF_REST));
        let mut out = Vec::new();
        let mut node = head.clone();
        while node.as_iri() != Some(RDF_NIL) {
            let mut firsts = self.objects(&node, &first);
            let mut rests = self.objects(&node, &rest);
            match (firsts.next(), rests.next(), firsts.next(), rests.next()) {
                (Some(f), Some(r), None, None) => {
                    out.push(f.clone());
                    node = r.clone();
                }
                _ => return Err(Error::Mapping(format!("{head} is not a well-formed list"))),
            }
            if out.len() > self.by_subject.len() {
                return Err(Error::Mapping(format!("{head} is a cyclic list")));
            }
        }
        Ok(out)
    }

    fn has_type(&self, s: &Term, class: &Term) -> bool {
        let t = Term::iri(RDF_TYPE);
        self.objects(s, &t).any(|o| o == class)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Position {
    Subject,
    Predicate,
    Object,
    Graph,
    Datatype,
    Language,
}

/// Builds the mapping from the triples of a mapping document.
pub fn mapping_from_triples(triples: Vec<Triple>) -> Result<Mapping> {
    let legacy = crate::legacy::uses_legacy(&triples);
    let triples = crate::legacy::translate(triples);
    // Vocabulary from modules RMLrs does not implement yet (IO targets, FNML,
    // CC, LV, star, other reference formulations) is reported as unsupported
    // before any structural check, so that a missing feature can never pass
    // as the "right" error.
    for t in &triples {
        for term in [&t.predicate, &t.object] {
            // IRIs under the namespace with a path (`rml:resources/...`, the
            // test data the specifications publish) are not vocabulary terms.
            if let Some(local) = term.as_iri().and_then(|i| i.strip_prefix(RML))
                && !local.contains('/')
                && !IMPLEMENTED.contains(&local)
            {
                return Err(Error::Unsupported(format!("rml:{local}")));
            }
        }
    }
    let mut g = Graph::new(triples);
    let logical_targets = collect_targets(&mut g)?;

    let tm_class = rml("TriplesMap");
    let nodes: Vec<Term> = g
        .order
        .iter()
        .filter(|s| {
            g.has_type(s, &tm_class)
                || g.objects(s, &rml("logicalSource")).next().is_some()
                || g.objects(s, &rml("subjectMap")).next().is_some()
        })
        .cloned()
        .collect();
    let index: HashMap<Term, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.clone(), i))
        .collect();

    let triples_maps = nodes
        .iter()
        .map(|n| triples_map(&g, n, &index))
        .collect::<Result<Vec<_>>>()?;
    // A logical view that no triples map uses must still be well-formed
    // (no cycles, no name collisions): the document declares it.
    for node in &g.order {
        if g.objects(node, &rml("viewOn")).next().is_some() {
            logical_view(&g, node, &mut Vec::new())?;
        }
    }
    let mut mapping = Mapping {
        triples_maps,
        logical_targets,
        legacy,
    };
    check_target_graphs(&mapping)?;
    if legacy {
        crate::legacy::absolute_json_paths(&mut mapping);
        crate::legacy::sparql_variables(&mut mapping)?;
    }
    Ok(mapping)
}

/// RML-IO, section 7.4: a mapping that sends the triples of a named graph
/// to a logical target whose serialization has no named graphs is invalid.
///
/// The triples of a predicate-object map (or the class triples of a subject
/// map) go to the targets of their subject, predicate and object maps in
/// every graph, and to a graph map's own targets in that graph. A graph map
/// that is not the constant `rml:defaultGraph` makes named graphs. Triples
/// this check does not see (collections of RML-CC) are refused by the target
/// writer when they arrive.
fn check_target_graphs(m: &Mapping) -> Result<()> {
    let default_graph = rml("defaultGraph");
    let named = |g: &&TermMap| !matches!(&g.expression, Some(Expression::Constant(t)) if *t == default_graph);
    for tm in m.triples_maps.iter().filter(|tm| tm.asserted) {
        let sm = &tm.subject_map;
        let mut groups: Vec<(Vec<&TermMap>, Vec<usize>)> = Vec::new();
        if !sm.classes.is_empty() {
            groups.push((sm.graph_maps.iter().collect(), sm.term_map.targets.clone()));
        }
        for pom in &tm.predicate_object_maps {
            let mut targets = sm.term_map.targets.clone();
            for pm in &pom.predicate_maps {
                targets.extend(&pm.targets);
            }
            for om in &pom.object_maps {
                if let ObjectMap::Term(t) = om {
                    targets.extend(&t.targets);
                }
            }
            groups.push((
                sm.graph_maps.iter().chain(&pom.graph_maps).collect(),
                targets,
            ));
        }
        for (graphs, targets) in groups {
            let named: Vec<&TermMap> = graphs.into_iter().filter(named).collect();
            if named.is_empty() {
                continue;
            }
            let all = targets.iter().chain(named.iter().flat_map(|g| &g.targets));
            for &i in all {
                let t = &m.logical_targets[i];
                if !t.serialization.has_named_graphs() {
                    return Err(Error::Mapping(format!(
                        "triples map {} sends named graphs to logical target \"{}\", \
                         whose serialization ({:?}) has no named graphs",
                        tm.node, t.path, t.serialization
                    )));
                }
            }
        }
    }
    Ok(())
}

const FORMATS: &str = "http://www.w3.org/ns/formats/";
const D2RQ: &str = "http://www.wiwiss.fu-berlin.de/suhl/bizer/D2RQ/0.1#";

/// A `d2rq:Database` source description.
fn database(g: &Graph, node: &Term) -> Result<DatabaseSource> {
    let text = |p: &str| -> Result<Option<String>> {
        let p = Term::iri(format!("{D2RQ}{p}"));
        let mut v: Vec<&Term> = g.objects(node, &p).collect();
        match v.pop() {
            None => Ok(None),
            Some(Term::Literal(l)) if v.is_empty() => Ok(Some(l.value.clone())),
            Some(_) => Err(Error::Mapping(format!(
                "{node}: d2rq property must be one string"
            ))),
        }
    };
    let dsn =
        text("jdbcDSN")?.ok_or_else(|| Error::Mapping(format!("{node} has no d2rq:jdbcDSN")))?;
    Ok(DatabaseSource {
        dsn,
        driver: text("jdbcDriver")?,
        username: text("username")?,
        password: text("password")?,
    })
}

/// Reads every node used as `rml:logicalTarget`. Effectively equal targets
/// (same properties, whatever their node) share one index.
const DCAT: &str = "http://www.w3.org/ns/dcat#";
const TD: &str = "https://www.w3.org/2019/wot/td#";
const HCTL: &str = "https://www.w3.org/2019/wot/hypermedia#";
const HTV: &str = "http://www.w3.org/2011/http#";
const SD: &str = "http://www.w3.org/ns/sparql-service-description#";

/// The objects of `node` for a predicate given by its full IRI.
fn objects_of(g: &Graph, node: &Term, p: &str) -> Vec<Term> {
    let p = Term::iri(p);
    g.objects(node, &p).cloned().collect()
}

/// Whether a source is described with DCAT or the Web of Things.
fn is_remote(g: &Graph, s: &Term) -> bool {
    [
        format!("{DCAT}downloadURL"),
        format!("{DCAT}accessURL"),
        format!("{TD}hasPropertyAffordance"),
    ]
    .iter()
    .any(|p| !objects_of(g, s, p).is_empty())
}

/// The one object of `node` for `p`. More than one is a mapping error when
/// `several_unsupported` is false, and not supported otherwise (the
/// specification does not say which one to use).
fn exactly_one(g: &Graph, node: &Term, p: &str, several_unsupported: bool) -> Result<Term> {
    let mut v = objects_of(g, node, p);
    match v.len() {
        1 => Ok(v.pop().expect("one")),
        0 => Err(Error::Mapping(format!("{node} has no <{p}>"))),
        _ if several_unsupported => Err(Error::Unsupported(format!("{node} with several <{p}>"))),
        _ => Err(Error::Mapping(format!("{node} has more than one <{p}>"))),
    }
}

/// An IRI or string naming a remote document; only HTTP(S) is read.
fn http_url(t: &Term) -> Result<String> {
    let url = match t {
        Term::Iri(i) => i.clone(),
        Term::Literal(l) => l.value.clone(),
        other => return Err(Error::Mapping(format!("{other} is not a URL"))),
    };
    let scheme = url.split_once(':').map(|(s, _)| s.to_ascii_lowercase());
    match scheme.as_deref() {
        Some("http" | "https") => Ok(url),
        Some(other) => Err(Error::Unsupported(format!(
            "{other} source {url} (only HTTP and HTTPS are read)"
        ))),
        None => Err(Error::Mapping(format!("{url:?} is not an absolute URL"))),
    }
}

/// A source described by a `dcat:Distribution` or a `td:Thing` (the RML-IO
/// registry): a document fetched with an HTTP GET.
fn remote(g: &Graph, s: &Term) -> Result<Source> {
    let download = format!("{DCAT}downloadURL");
    if !objects_of(g, s, &download).is_empty() {
        let url = match exactly_one(g, s, &download, false)? {
            t @ Term::Iri(_) => http_url(&t)?,
            other => {
                return Err(Error::Mapping(format!(
                    "dcat:downloadURL must be an IRI, got {other}"
                )));
            }
        };
        return Ok(Source::Remote(RemoteSource {
            url,
            content_type: None,
            headers: Vec::new(),
        }));
    }
    if !objects_of(g, s, &format!("{DCAT}accessURL")).is_empty() {
        return Err(Error::Unsupported(
            "dcat:accessURL (a data service); only dcat:downloadURL is read".into(),
        ));
    }
    let affordance = exactly_one(g, s, &format!("{TD}hasPropertyAffordance"), true)?;
    let form = exactly_one(g, &affordance, &format!("{TD}hasForm"), true)?;
    // A string property: required, or `None` when absent.
    let text = |node: &Term, p: String, required: bool| -> Result<Option<String>> {
        if !required && objects_of(g, node, &p).is_empty() {
            return Ok(None);
        }
        match exactly_one(g, node, &p, false)? {
            Term::Literal(l) => Ok(Some(l.value)),
            other => Err(Error::Mapping(format!(
                "<{p}> must be a string, got {other}"
            ))),
        }
    };
    let content_type = text(&form, format!("{HCTL}forContentType"), false)?;
    // Streams are read by subscribing, not by one request; the caller feeds
    // their messages in instead.
    if content_type.as_deref() == Some("text/event-stream") {
        return Err(Error::Unsupported("server-sent event streams".into()));
    }
    let url = http_url(&exactly_one(g, &form, &format!("{HCTL}hasTarget"), false)?)?;
    if let Some(method) = text(&form, format!("{HTV}methodName"), false)?
        && !method.eq_ignore_ascii_case("GET")
    {
        return Err(Error::Unsupported(format!("htv:methodName {method:?}")));
    }
    let mut headers = Vec::new();
    for list in objects_of(g, &form, &format!("{HTV}headers")) {
        for h in g.list(&list)? {
            let name = text(&h, format!("{HTV}fieldName"), true)?.expect("required");
            let value = text(&h, format!("{HTV}fieldValue"), true)?.expect("required");
            headers.push((name, value));
        }
    }
    Ok(Source::Remote(RemoteSource {
        url,
        content_type,
        headers,
    }))
}

fn collect_targets(g: &mut Graph) -> Result<Vec<LogicalTarget>> {
    let lt = rml("logicalTarget");
    let mut nodes = Vec::new();
    for s in &g.order {
        for o in g.objects(s, &lt) {
            if !nodes.contains(o) {
                nodes.push(o.clone());
            }
        }
    }
    let mut targets: Vec<LogicalTarget> = Vec::new();
    let mut index = HashMap::new();
    for node in nodes {
        let t = logical_target(g, &node)?;
        let i = match targets.iter().position(|x| *x == t) {
            Some(i) => i,
            None => {
                targets.push(t);
                targets.len() - 1
            }
        };
        index.insert(node, i);
    }
    g.target_index = index;
    Ok(targets)
}

fn logical_target(g: &Graph, node: &Term) -> Result<LogicalTarget> {
    let target = g
        .one(node, "target")?
        .ok_or_else(|| Error::Mapping(format!("logical target {node} has no rml:target")))?;
    let path = match g.one(&target, "path")? {
        Some(Term::Literal(l)) => l.value,
        Some(other) => {
            return Err(Error::Mapping(format!(
                "rml:path must be a string, got {other}"
            )));
        }
        None => {
            return Err(Error::Unsupported(format!(
                "target without rml:path: {}",
                describe(g, &target)
            )));
        }
    };
    let root = match g.one(&target, "root")? {
        Some(r) if r == rml("MappingDirectory") => PathRoot::MappingDirectory,
        None | Some(_) => PathRoot::CurrentWorkingDirectory,
    };
    let serialization = match g
        .one(node, "serialization")?
        .as_ref()
        .and_then(Term::as_iri)
    {
        None => Serialization::NQuads,
        Some(f) => match f.strip_prefix(FORMATS) {
            Some("N-Quads") => Serialization::NQuads,
            Some("N-Triples") => Serialization::NTriples,
            Some("Turtle") => Serialization::Turtle,
            Some("N3") => Serialization::N3,
            Some("RDF_JSON") => Serialization::RdfJson,
            Some("JSON-LD") => Serialization::JsonLd,
            Some("RDF_XML") => Serialization::RdfXml,
            _ => return Err(Error::Unsupported(format!("target serialization <{f}>"))),
        },
    };
    let mut encoding = TextEncoding::Utf8;
    for n in [node, &target] {
        match g.one(n, "encoding")? {
            None => {}
            Some(e) if e == rml("UTF-8") => encoding = TextEncoding::Utf8,
            Some(e) if e == rml("UTF-16") => encoding = TextEncoding::Utf16,
            Some(e) => return Err(Error::Unsupported(format!("target encoding {e}"))),
        }
    }
    // The specification puts rml:compression on the rml:Target; the test
    // cases put it on the rml:LogicalTarget. Both are read.
    let mut compression = Compression::None;
    for n in [node, &target] {
        if let Some(c) = g.one(n, "compression")? {
            compression = compression_of(&c)?;
        }
    }
    Ok(LogicalTarget {
        root,
        path,
        serialization,
        encoding,
        compression,
    })
}

fn compression_of(c: &Term) -> Result<Compression> {
    Ok(match c.as_iri().and_then(|i| i.strip_prefix(RML)) {
        Some("none") => Compression::None,
        Some("gzip") => Compression::Gzip,
        Some("zip") => Compression::Zip,
        Some("targz") => Compression::TarGz,
        Some("tarxz") => Compression::TarXz,
        _ => return Err(Error::Unsupported(format!("compression {c}"))),
    })
}

/// The target indices named by `rml:logicalTarget` on a term map node.
fn targets_of(g: &Graph, node: &Term) -> Vec<usize> {
    let mut out: Vec<usize> = g
        .objects(node, &rml("logicalTarget"))
        .filter_map(|t| g.target_index.get(t).copied())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn triples_map(g: &Graph, node: &Term, index: &HashMap<Term, usize>) -> Result<TriplesMap> {
    let ls = g
        .one(node, "logicalSource")?
        .ok_or_else(|| Error::Mapping(format!("{node} has no rml:logicalSource")))?;
    let logical_source = logical_source(g, &ls, &mut Vec::new())?;

    let mut subjects: Vec<TermMap> = Vec::new();
    for sm in g.all(node, "subjectMap") {
        subjects.push(gathering_term_map(g, &sm, Position::Subject, index)?);
    }
    for c in g.all(node, "subject") {
        subjects.push(constant(c, Position::Subject)?);
    }
    let subject_node = g.all(node, "subjectMap").pop();
    let term_map_ = match subjects.len() {
        1 => subjects.pop().expect("one"),
        0 => return Err(Error::Mapping(format!("{node} has no subject map"))),
        _ => {
            return Err(Error::Mapping(format!(
                "{node} has more than one subject map"
            )));
        }
    };
    let (classes, graph_maps) = match &subject_node {
        Some(sm) => (iris(g, sm, "class")?, graph_maps(g, sm)?),
        None => (Vec::new(), Vec::new()),
    };
    let subject_map = SubjectMap {
        term_map: term_map_,
        classes,
        graph_maps,
    };

    let predicate_object_maps = g
        .all(node, "predicateObjectMap")
        .iter()
        .map(|pom| predicate_object_map(g, pom, index))
        .collect::<Result<Vec<_>>>()?;

    let base_iri = match g.one(node, "baseIRI")? {
        Some(Term::Iri(i)) => Some(i),
        Some(other) => {
            return Err(Error::Mapping(format!(
                "rml:baseIRI must be an IRI, got {other}"
            )));
        }
        None => None,
    };

    let non_asserted = g.has_type(node, &rml("NonAssertedTriplesMap"));
    if non_asserted && g.has_type(node, &rml("AssertedTriplesMap")) {
        return Err(Error::Mapping(format!(
            "{node} is both an asserted and a non-asserted triples map"
        )));
    }

    Ok(TriplesMap {
        node: node.clone(),
        logical_source,
        subject_map,
        predicate_object_maps,
        base_iri,
        asserted: !non_asserted,
    })
}

fn iris(g: &Graph, node: &Term, p: &str) -> Result<Vec<String>> {
    g.all(node, p)
        .into_iter()
        .map(|t| match t {
            Term::Iri(i) => Ok(i),
            other => Err(Error::Mapping(format!(
                "rml:{p} must be an IRI, got {other}"
            ))),
        })
        .collect()
}

/// A logical source or logical view. `views` holds the views being read,
/// to reject a view that depends on itself.
fn logical_source(g: &Graph, node: &Term, views: &mut Vec<Term>) -> Result<LogicalSource> {
    if g.one(node, "viewOn")?.is_some() || g.has_type(node, &rml("LogicalView")) {
        return logical_view(g, node, views);
    }
    let source = match g.one(node, "source")? {
        None => return Err(Error::Mapping("logical source without rml:source".into())),
        Some(Term::Literal(l)) => Source::Path(l.value),
        Some(Term::Iri(i)) if i == crate::legacy::PROCESSOR_DATABASE => Source::ProcessorDatabase,
        Some(s)
            if g.objects(&s, &Term::iri(format!("{D2RQ}jdbcDSN")))
                .next()
                .is_some() =>
        {
            Source::Database(database(g, &s)?)
        }
        Some(s) if g.has_type(&s, &csvw("Table")) || csvw_one(g, &s, "url")?.is_some() => {
            Source::CsvwTable(csvw_table(g, &s)?)
        }
        Some(s) if is_remote(g, &s) => remote(g, &s)?,
        Some(s) if !objects_of(g, &s, &format!("{SD}endpoint")).is_empty() => {
            match exactly_one(g, &s, &format!("{SD}endpoint"), false)? {
                t @ Term::Iri(_) => Source::SparqlEndpoint(http_url(&t)?),
                other => {
                    return Err(Error::Mapping(format!(
                        "sd:endpoint must be an IRI, got {other}"
                    )));
                }
            }
        }
        Some(s) => match g.one(&s, "path")? {
            Some(Term::Literal(path)) => {
                let root = match g.one(&s, "root")? {
                    Some(r) if r == rml("MappingDirectory") => PathRoot::MappingDirectory,
                    Some(r) if r == rml("CurrentWorkingDirectory") => {
                        PathRoot::CurrentWorkingDirectory
                    }
                    None => PathRoot::CurrentWorkingDirectory,
                    Some(r) => return Err(Error::Unsupported(format!("rml:root {r}"))),
                };
                let compression = match g.one(&s, "compression")? {
                    Some(c) => compression_of(&c)?,
                    None => Compression::None,
                };
                Source::RelativePath {
                    root,
                    path: path.value,
                    compression,
                    encoding: rml_encoding(g, &s)?.unwrap_or_default(),
                }
            }
            Some(other) => {
                return Err(Error::Mapping(format!(
                    "rml:path must be a string, got {other}"
                )));
            }
            None => Source::Other(describe(g, &s)),
        },
    };
    let reference_formulation = reference_formulation(g, node)?;
    // rml:null takes precedence over csvw:null (RML-IO, Source).
    let null_values = match g.one(node, "source")? {
        Some(src @ (Term::Iri(_) | Term::BlankNode(_))) => {
            let mut nulls = g.all(&src, "null");
            if nulls.is_empty() {
                nulls = csvw_all(g, &src, "null");
            }
            nulls
                .into_iter()
                .map(|t| match t {
                    Term::Literal(l) => Ok(l.value),
                    other => Err(Error::Mapping(format!(
                        "rml:null and csvw:null must be strings, got {other}"
                    ))),
                })
                .collect::<Result<Vec<_>>>()?
        }
        _ => Vec::new(),
    };
    let iterator = match g.one(node, "iterator")? {
        None => None,
        Some(Term::Literal(l)) => Some(l.value),
        Some(other) => {
            return Err(Error::Mapping(format!(
                "rml:iterator must be a string, got {other}"
            )));
        }
    };
    Ok(LogicalSource {
        source,
        reference_formulation,
        iterator,
        null_values,
    })
}

/// `rml:encoding` of a file source.
fn rml_encoding(g: &Graph, node: &Term) -> Result<Option<SourceEncoding>> {
    Ok(match g.one(node, "encoding")? {
        None => None,
        Some(e) if e == rml("UTF-8") => Some(SourceEncoding::Utf8),
        // Without a byte order mark, UTF-16 is big-endian (RFC 2781, 4.3).
        Some(e) if e == rml("UTF-16") => Some(SourceEncoding::Utf16Be),
        Some(e) => return Err(Error::Unsupported(format!("source encoding {e}"))),
    })
}

pub const CSVW: &str = "http://www.w3.org/ns/csvw#";

fn csvw(local: &str) -> Term {
    Term::Iri(format!("{CSVW}{local}"))
}

fn csvw_all(g: &Graph, node: &Term, p: &str) -> Vec<Term> {
    let p = csvw(p);
    g.objects(node, &p).cloned().collect()
}

fn csvw_one(g: &Graph, node: &Term, p: &str) -> Result<Option<Term>> {
    let mut v = csvw_all(g, node, p);
    match v.len() {
        0 => Ok(None),
        1 => Ok(v.pop()),
        _ => Err(Error::Mapping(format!("{node} has more than one csvw:{p}"))),
    }
}

/// A `csvw:Table` source (RML-IO-Registry, CSVW): `csvw:url` and the
/// `csvw:dialect` that says how to read it. Properties of the `rml:Source`
/// (`rml:encoding`, `rml:compression`) take precedence.
fn csvw_table(g: &Graph, node: &Term) -> Result<CsvwTable> {
    if csvw_one(g, node, "tableSchema")?.is_some() {
        return Err(Error::Unsupported("csvw:tableSchema".into()));
    }
    let url = match csvw_one(g, node, "url")? {
        Some(Term::Literal(l)) => l.value,
        Some(Term::Iri(i)) => i,
        Some(other) => {
            return Err(Error::Mapping(format!(
                "csvw:url must be a URL, got {other}"
            )));
        }
        None => {
            return Err(Error::Mapping(format!("csvw:Table {node} has no csvw:url")));
        }
    };
    let (dialect, csvw_encoding) = match csvw_one(g, node, "dialect")? {
        Some(d) => csvw_dialect(g, &d)?,
        None => (CsvDialect::default(), None),
    };
    let compression = match g.one(node, "compression")? {
        Some(c) => compression_of(&c)?,
        None => Compression::None,
    };
    let encoding = rml_encoding(g, node)?.or(csvw_encoding).unwrap_or_default();
    Ok(CsvwTable {
        url,
        compression,
        encoding,
        dialect,
    })
}

/// The dialect properties that are given. Those that are not keep the
/// plain RFC 4180 reading of `rml:CSV`, not the CSVW defaults: CSVW itself
/// is not consistent about them (its `trim` default is `true` in the
/// property definition and `false` in the example default dialect), and
/// RML-IO says that CSV has no NULL value unless one is declared.
fn csvw_dialect(g: &Graph, node: &Term) -> Result<(CsvDialect, Option<SourceEncoding>)> {
    let mut d = CsvDialect::default();
    let mut encoding = None;
    let mut header = None;
    let mut header_row_count = None;
    let mut trim = None;
    let mut skip_initial_space = None;
    let Some(props) = g.by_subject.get(node) else {
        return Ok((d, encoding));
    };
    let mut seen = Vec::new();
    for (p, o) in props {
        let Some(local) = p.as_iri().and_then(|i| i.strip_prefix(CSVW)) else {
            continue;
        };
        if seen.contains(&local) {
            return Err(Error::Mapping(format!(
                "{node} has more than one csvw:{local}"
            )));
        }
        seen.push(local);
        let text = || match o {
            Term::Literal(l) => Ok(l.value.as_str()),
            other => Err(Error::Mapping(format!(
                "csvw:{local} must be a literal, got {other}"
            ))),
        };
        let one_char = || -> Result<char> {
            let t = text()?;
            let mut chars = t.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Ok(c),
                (None, _) => Err(Error::Mapping(format!("csvw:{local} is empty"))),
                _ => Err(Error::Unsupported(format!(
                    "csvw:{local} longer than one character ({t:?})"
                ))),
            }
        };
        let boolean = || -> Result<bool> {
            match text()? {
                "true" | "1" => Ok(true),
                "false" | "0" => Ok(false),
                t => Err(Error::Mapping(format!(
                    "csvw:{local} must be a boolean, got {t:?}"
                ))),
            }
        };
        let count = || -> Result<usize> {
            let t = text()?;
            t.parse().map_err(|_| {
                Error::Mapping(format!(
                    "csvw:{local} must be a non-negative integer, got {t:?}"
                ))
            })
        };
        match local {
            "delimiter" => d.delimiter = one_char()?,
            "quoteChar" => d.quote = one_char()?,
            "doubleQuote" => d.double_quote = boolean()?,
            "commentPrefix" => {
                let t = text()?;
                d.comment_prefix = (!t.is_empty()).then(|| t.to_owned());
            }
            "header" => header = Some(boolean()?),
            "headerRowCount" => header_row_count = Some(count()?),
            "skipRows" => d.skip_rows = count()?,
            "skipColumns" => d.skip_columns = count()?,
            "skipInitialSpace" => skip_initial_space = Some(boolean()?),
            "trim" => {
                trim = Some(match text()? {
                    "true" | "1" => Trim::Both,
                    "false" | "0" => Trim::None,
                    "start" => Trim::Start,
                    "end" => Trim::End,
                    t => {
                        return Err(Error::Mapping(format!(
                            "csvw:trim must be true, false, start or end, got {t:?}"
                        )));
                    }
                })
            }
            "encoding" => encoding = Some(csvw_encoding(text()?)?),
            // Blank rows are always skipped.
            "skipBlankRows" => {
                boolean()?;
            }
            // Lines end with CRLF or LF, which are the CSVW defaults.
            "lineTerminators" if matches!(text()?, "\n" | "\r\n") => {}
            other => return Err(Error::Unsupported(format!("csvw:{other}"))),
        }
    }
    // headerRowCount wins over header; trim wins over skipInitialSpace.
    if let Some(n) = header_row_count.or(header.map(usize::from)) {
        d.header_rows = n;
    }
    let initial = skip_initial_space.map(|s| if s { Trim::Start } else { Trim::None });
    if let Some(t) = trim.or(initial) {
        d.trim = t;
    }
    if d.delimiter == d.quote {
        return Err(Error::Mapping(
            "csvw:delimiter and csvw:quoteChar are the same character".into(),
        ));
    }
    Ok((d, encoding))
}

/// A `csvw:encoding` label. CSVW takes its labels from the WHATWG Encoding
/// standard, where `utf-16` is little-endian unless a byte order mark says
/// otherwise.
fn csvw_encoding(label: &str) -> Result<SourceEncoding> {
    Ok(match label.trim().to_ascii_lowercase().as_str() {
        "utf-8" | "utf8" | "unicode-1-1-utf-8" | "unicode11utf8" | "unicode20utf8"
        | "x-unicode20utf8" => SourceEncoding::Utf8,
        "utf-16" | "utf-16le" | "csunicode" | "iso-10646-ucs-2" | "ucs-2" | "unicode"
        | "unicodefeff" => SourceEncoding::Utf16Le,
        "utf-16be" | "unicodefffe" => SourceEncoding::Utf16Be,
        _ => return Err(Error::Unsupported(format!("csvw:encoding {label:?}"))),
    })
}

/// The `rml:namespace` declarations of an `rml:XPathReferenceFormulation`.
fn xpath_namespaces(g: &Graph, node: &Term) -> Result<Vec<(String, String)>> {
    let text = |n: &Term, p: &str| -> Result<String> {
        match g.one(n, p)? {
            Some(Term::Literal(l)) => Ok(l.value),
            Some(other) => Err(Error::Mapping(format!(
                "rml:{p} must be a string, got {other}"
            ))),
            None => Err(Error::Mapping(format!("namespace {n} has no rml:{p}"))),
        }
    };
    let mut out = Vec::new();
    for ns in g.all(node, "namespace") {
        out.push((text(&ns, "namespacePrefix")?, text(&ns, "namespaceURL")?));
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// `rml:referenceFormulation` of a logical source or iterable field.
fn reference_formulation(g: &Graph, node: &Term) -> Result<Option<ReferenceFormulation>> {
    Ok(match g.one(node, "referenceFormulation")? {
        None => None,
        Some(r) if r == rml("JSONPath") => Some(ReferenceFormulation::JsonPath),
        Some(r) if r == rml("CSV") => Some(ReferenceFormulation::Csv),
        Some(r) if r == rml("SQL2008Table") || r == rml("SQLTable") => {
            Some(ReferenceFormulation::SqlTable)
        }
        Some(r) if r == rml("SQL2008Query") || r == rml("SQLQuery") => {
            Some(ReferenceFormulation::SqlQuery)
        }
        Some(r) if r == rml("XPath") => Some(ReferenceFormulation::XPath {
            namespaces: Vec::new(),
        }),
        Some(r) if g.has_type(&r, &rml("XPathReferenceFormulation")) => {
            Some(ReferenceFormulation::XPath {
                namespaces: xpath_namespaces(g, &r)?,
            })
        }
        Some(Term::Iri(r)) if r.starts_with(FORMATS) => {
            let format = match &r[FORMATS.len()..] {
                "SPARQL_Results_CSV" => SparqlResultsFormat::Csv,
                "SPARQL_Results_TSV" => SparqlResultsFormat::Tsv,
                "SPARQL_Results_JSON" => SparqlResultsFormat::Json,
                "SPARQL_Results_XML" => SparqlResultsFormat::Xml,
                _ => return Ok(Some(ReferenceFormulation::Other(format!("<{r}>")))),
            };
            Some(ReferenceFormulation::SparqlResults(format))
        }
        Some(r) => Some(ReferenceFormulation::Other(r.to_string())),
    })
}

fn string_of(g: &Graph, node: &Term, p: &str) -> Result<Option<String>> {
    match g.one(node, p)? {
        None => Ok(None),
        Some(Term::Literal(l)) => Ok(Some(l.value)),
        Some(other) => Err(Error::Mapping(format!(
            "rml:{p} must be a string, got {other}"
        ))),
    }
}

/// A logical view (RML-LV), as the logical source of the views and triples
/// maps that use it.
fn logical_view(g: &Graph, node: &Term, views: &mut Vec<Term>) -> Result<LogicalSource> {
    if views.contains(node) {
        return Err(Error::Mapping(format!(
            "logical view {node} depends on itself"
        )));
    }
    if g.one(node, "referenceFormulation")?.is_some() || g.one(node, "iterator")?.is_some() {
        return Err(Error::Mapping(format!(
            "logical view {node} cannot declare a reference formulation or iterator"
        )));
    }
    views.push(node.clone());
    let view_on = g
        .one(node, "viewOn")?
        .ok_or_else(|| Error::Mapping(format!("logical view {node} has no rml:viewOn")))?;
    let view_on = logical_source(g, &view_on, views)?;
    let fields = read_fields(g, node, &mut Vec::new())?;
    if fields.is_empty() {
        return Err(Error::Mapping(format!("logical view {node} has no field")));
    }
    let mut names = Vec::new();
    absolute_names(&fields, "", &mut names);
    let mut joins = Vec::new();
    for (property, inner) in [("leftJoin", false), ("innerJoin", true)] {
        for j in g.all(node, property) {
            let parent = g
                .one(&j, "parentLogicalView")?
                .ok_or_else(|| Error::Mapping(format!("{j} has no rml:parentLogicalView")))?;
            let parent = logical_source(g, &parent, views)?;
            if !matches!(parent.source, Source::View(_)) {
                return Err(Error::Mapping(format!(
                    "the parent of {j} is not a logical view"
                )));
            }
            let conditions = g
                .all(&j, "joinCondition")
                .iter()
                .map(|jc| join_condition(g, jc))
                .collect::<Result<Vec<_>>>()?;
            if conditions.is_empty() {
                return Err(Error::Mapping(format!("{j} has no join condition")));
            }
            let fields = read_fields(g, &j, &mut Vec::new())?;
            for f in &fields {
                if !matches!(f.kind, FieldKind::Expression(_)) || !f.fields.is_empty() {
                    return Err(Error::Mapping(format!(
                        "field {:?} of {j} must be an expression field without nested fields",
                        f.name
                    )));
                }
                if names.contains(&f.name) {
                    return Err(Error::Mapping(format!(
                        "field name {:?} of {j} is already used in logical view {node}",
                        f.name
                    )));
                }
                names.push(f.name.clone());
            }
            joins.push(ViewJoin {
                inner,
                parent,
                conditions,
                fields,
            });
        }
    }
    views.pop();
    Ok(LogicalSource {
        source: Source::View(Box::new(LogicalView {
            view_on,
            fields,
            joins,
        })),
        reference_formulation: None,
        iterator: None,
        null_values: Vec::new(),
    })
}

fn absolute_names(fields: &[Field], prefix: &str, out: &mut Vec<String>) {
    for f in fields {
        let name = if prefix.is_empty() {
            f.name.clone()
        } else {
            format!("{prefix}.{}", f.name)
        };
        absolute_names(&f.fields, &name, out);
        out.push(name);
    }
}

/// The fields (`rml:field`) of a view, join or field. `path` holds the
/// fields being read, to reject a field nested in itself.
fn read_fields(g: &Graph, node: &Term, path: &mut Vec<Term>) -> Result<Vec<Field>> {
    let mut out: Vec<Field> = Vec::new();
    for f in g.all(node, "field") {
        if path.contains(&f) {
            return Err(Error::Mapping(format!("field {f} is nested in itself")));
        }
        path.push(f.clone());
        let name = string_of(g, &f, "fieldName")?
            .ok_or_else(|| Error::Mapping(format!("field {f} has no rml:fieldName")))?;
        if out.iter().any(|o| o.name == name) {
            return Err(Error::Mapping(format!(
                "two fields of {node} are named {name:?}"
            )));
        }
        let kind = match (expression(g, &f)?, g.has_type(&f, &rml("IterableField"))) {
            (Some(_), true) => {
                return Err(Error::Mapping(format!(
                    "iterable field {f} has an expression"
                )));
            }
            (Some(e), false) => FieldKind::Expression(e),
            (None, _) => FieldKind::Iterable {
                reference_formulation: reference_formulation(g, &f)?,
                iterator: string_of(g, &f, "iterator")?,
            },
        };
        let nested = read_fields(g, &f, path)?;
        path.pop();
        out.push(Field {
            name,
            kind,
            fields: nested,
        });
    }
    Ok(out)
}

/// A short description of a source node, for "not supported" messages.
fn describe(g: &Graph, node: &Term) -> String {
    let t = Term::iri(RDF_TYPE);
    g.objects(node, &t)
        .map(|o| o.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn graph_maps(g: &Graph, node: &Term) -> Result<Vec<TermMap>> {
    let mut out = Vec::new();
    for gm in g.all(node, "graphMap") {
        out.push(term_map(g, &gm, Position::Graph)?);
    }
    for c in g.all(node, "graph") {
        out.push(constant(c, Position::Graph)?);
    }
    Ok(out)
}

fn predicate_object_map(
    g: &Graph,
    node: &Term,
    index: &HashMap<Term, usize>,
) -> Result<PredicateObjectMap> {
    let mut predicate_maps = Vec::new();
    for pm in g.all(node, "predicateMap") {
        predicate_maps.push(term_map(g, &pm, Position::Predicate)?);
    }
    for c in g.all(node, "predicate") {
        predicate_maps.push(constant(c, Position::Predicate)?);
    }
    if predicate_maps.is_empty() {
        return Err(Error::Mapping(format!(
            "predicate-object map {node} has no predicate map"
        )));
    }

    let mut object_maps = Vec::new();
    for om in g.all(node, "objectMap") {
        object_maps.push(object_map(g, &om, index)?);
    }
    for c in g.all(node, "object") {
        object_maps.push(ObjectMap::Term(constant(c, Position::Object)?));
    }
    if object_maps.is_empty() {
        return Err(Error::Mapping(format!(
            "predicate-object map {node} has no object map"
        )));
    }

    Ok(PredicateObjectMap {
        predicate_maps,
        object_maps,
        graph_maps: graph_maps(g, node)?,
    })
}

/// An object map node: a referencing object map or a (gathering) term map.
fn object_map(g: &Graph, om: &Term, index: &HashMap<Term, usize>) -> Result<ObjectMap> {
    let Some(parent) = g.one(om, "parentTriplesMap")? else {
        return Ok(ObjectMap::Term(gathering_term_map(
            g,
            om,
            Position::Object,
            index,
        )?));
    };
    let parent = *index.get(&parent).ok_or_else(|| {
        Error::Mapping(format!(
            "rml:parentTriplesMap {parent} is not a triples map"
        ))
    })?;
    let join_conditions = g
        .all(om, "joinCondition")
        .iter()
        .map(|jc| join_condition(g, jc))
        .collect::<Result<Vec<_>>>()?;
    Ok(ObjectMap::Ref(RefObjectMap {
        parent,
        join_conditions,
    }))
}

/// A star map (RML-star): the triples of another triples map as quoted triples.
fn star_map(
    g: &Graph,
    node: &Term,
    quoted: &Term,
    index: &HashMap<Term, usize>,
) -> Result<TermMap> {
    let parent = *index.get(quoted).ok_or_else(|| {
        Error::Mapping(format!(
            "rml:quotedTriplesMap {quoted} is not a triples map"
        ))
    })?;
    for p in [
        "constant",
        "reference",
        "template",
        "functionExecution",
        "gather",
        "termType",
    ] {
        if g.one(node, p)?.is_some() {
            return Err(Error::Mapping(format!(
                "star map {node} cannot also have rml:{p}"
            )));
        }
    }
    let join_conditions = g
        .all(node, "joinCondition")
        .iter()
        .map(|jc| join_condition(g, jc))
        .collect::<Result<Vec<_>>>()?;
    Ok(TermMap {
        expression: None,
        term_type: TermType::Iri,
        datatype_map: None,
        language_map: None,
        condition: condition(g, node)?,
        targets: targets_of(g, node),
        gather: None,
        quoted: Some(Box::new(RefObjectMap {
            parent,
            join_conditions,
        })),
    })
}

/// A subject or object term map, with its gather map if it has one (RML-CC).
fn gathering_term_map(
    g: &Graph,
    node: &Term,
    position: Position,
    index: &HashMap<Term, usize>,
) -> Result<TermMap> {
    if let Some(quoted) = g.one(node, "quotedTriplesMap")? {
        return star_map(g, node, &quoted, index);
    }
    let mut m = term_map(g, node, position)?;
    let Some(list) = g.one(node, "gather")? else {
        return Ok(m);
    };
    let items = g
        .list(&list)?
        .iter()
        .map(|item| object_map(g, item, index))
        .collect::<Result<Vec<_>>>()?;
    if items.is_empty() {
        return Err(Error::Mapping(format!("{node} gathers an empty list")));
    }
    let gather_as = match g.one(node, "gatherAs")? {
        Some(c) => match c.as_iri().and_then(|i| i.strip_prefix(RDF)) {
            Some("List") => Collection::List,
            Some("Bag") => Collection::Bag,
            Some("Seq") => Collection::Seq,
            Some("Alt") => Collection::Alt,
            _ => return Err(Error::Mapping(format!("unknown rml:gatherAs {c}"))),
        },
        None => return Err(Error::Mapping(format!("{node} has no rml:gatherAs"))),
    };
    let strategy = match g.one(node, "strategy")? {
        None => Strategy::Append,
        Some(s) => match s.as_iri().and_then(|i| i.strip_prefix(RML)) {
            Some("append") => Strategy::Append,
            Some("cartesianProduct") => Strategy::CartesianProduct,
            _ => return Err(Error::Mapping(format!("unknown rml:strategy {s}"))),
        },
    };
    let allow_empty = match g.one(node, "allowEmptyListAndContainer")? {
        None => false,
        Some(Term::Literal(l)) if l.value == "true" || l.value == "1" => true,
        Some(Term::Literal(l)) if l.value == "false" || l.value == "0" => false,
        Some(other) => {
            return Err(Error::Mapping(format!(
                "rml:allowEmptyListAndContainer must be a boolean, got {other}"
            )));
        }
    };
    m.gather = Some(Box::new(GatherMap {
        items,
        gather_as,
        strategy,
        allow_empty,
    }));
    Ok(m)
}

fn join_condition(g: &Graph, node: &Term) -> Result<JoinCondition> {
    let side = |map: &str, shortcut: &str| -> Result<Expression> {
        let mut found = Vec::new();
        for m in g.all(node, map) {
            found.push(
                expression(g, &m)?
                    .ok_or_else(|| Error::Mapping(format!("rml:{map} without expression")))?,
            );
        }
        for r in g.all(node, shortcut) {
            match r {
                Term::Literal(l) => found.push(Expression::Reference(l.value)),
                other => {
                    return Err(Error::Mapping(format!(
                        "rml:{shortcut} must be a string, got {other}"
                    )));
                }
            }
        }
        match found.len() {
            1 => Ok(found.pop().expect("one")),
            0 => Err(Error::Mapping(format!("join condition without rml:{map}"))),
            _ => Err(Error::Mapping(format!(
                "join condition with more than one rml:{map}"
            ))),
        }
    };
    Ok(JoinCondition {
        child: side("childMap", "child")?,
        parent: side("parentMap", "parent")?,
    })
}

/// The expression of an expression map, if it has one.
fn expression(g: &Graph, node: &Term) -> Result<Option<Expression>> {
    let mut found = Vec::new();
    if let Some(c) = g.one(node, "constant")? {
        found.push(Expression::Constant(c));
    }
    if let Some(r) = g.one(node, "reference")? {
        match r {
            Term::Literal(l) => found.push(Expression::Reference(l.value)),
            other => {
                return Err(Error::Mapping(format!(
                    "rml:reference must be a string, got {other}"
                )));
            }
        }
    }
    if let Some(t) = g.one(node, "template")? {
        match t {
            Term::Literal(l) => found.push(Expression::Template(template::parse(&l.value)?)),
            other => {
                return Err(Error::Mapping(format!(
                    "rml:template must be a string, got {other}"
                )));
            }
        }
    }
    if let Some(exec) = g.one(node, "functionExecution")? {
        let output = constant_iri(g, node, "return", "returnMap")?;
        found.push(Expression::Function(Box::new(function_call(
            g, &exec, output,
        )?)));
    } else if g.objects(node, &rml("returnMap")).next().is_some()
        || g.objects(node, &rml("return")).next().is_some()
    {
        return Err(Error::Mapping(format!(
            "{node} has rml:return but no rml:functionExecution"
        )));
    }
    match found.len() {
        0 => Ok(None),
        1 => Ok(found.pop()),
        _ => Err(Error::Mapping(format!(
            "{node} has more than one of rml:constant, rml:reference, rml:template"
        ))),
    }
}

/// An IRI given by a constant shortcut (`rml:function ex:f`) or by a map
/// with a constant (`rml:functionMap [ rml:constant ex:f ]`).
fn constant_iri(g: &Graph, node: &Term, shortcut: &str, map: &str) -> Result<Option<String>> {
    let mut found = Vec::new();
    for t in g.all(node, shortcut) {
        found.push(t);
    }
    for m in g.all(node, map) {
        match expression(g, &m)? {
            Some(Expression::Constant(t)) => found.push(t),
            Some(_) => {
                return Err(Error::Unsupported(format!(
                    "non-constant rml:{map} (RML-FNML)"
                )));
            }
            None => return Err(Error::Mapping(format!("rml:{map} {m} has no rml:constant"))),
        }
    }
    match found.len() {
        0 => Ok(None),
        1 => match found.pop().expect("one") {
            Term::Iri(i) => Ok(Some(i)),
            other => Err(Error::Mapping(format!(
                "rml:{shortcut} must be an IRI, got {other}"
            ))),
        },
        _ => Err(Error::Mapping(format!(
            "{node} has more than one rml:{shortcut}"
        ))),
    }
}

/// A function execution node (RML-FNML) and the output to take from it.
fn function_call(g: &Graph, exec: &Term, output: Option<String>) -> Result<FunctionCall> {
    let function = constant_iri(g, exec, "function", "functionMap")?
        .ok_or_else(|| Error::Mapping(format!("function execution {exec} has no rml:function")))?;
    let mut inputs = Vec::new();
    for input in g.all(exec, "input") {
        let parameter = constant_iri(g, &input, "parameter", "parameterMap")?
            .ok_or_else(|| Error::Mapping(format!("input {input} has no rml:parameter")))?;
        let mut values = Vec::new();
        for v in g.all(&input, "inputValue") {
            values.push(Expression::Constant(v));
        }
        for m in g.all(&input, "inputValueMap") {
            values.push(expression(g, &m)?.ok_or_else(|| {
                Error::Mapping(format!("rml:inputValueMap {m} has no expression"))
            })?);
        }
        match values.len() {
            1 => inputs.push((parameter, values.pop().expect("one"))),
            0 => return Err(Error::Mapping(format!("input {input} has no value"))),
            _ => {
                return Err(Error::Mapping(format!(
                    "input {input} has more than one value"
                )));
            }
        }
    }
    Ok(FunctionCall {
        function,
        inputs,
        output,
    })
}

/// `rml:condition` of an expression map: a function execution whose output
/// must be `true` for the map to produce anything.
fn condition(g: &Graph, node: &Term) -> Result<Option<FunctionCall>> {
    let Some(c) = g.one(node, "condition")? else {
        return Ok(None);
    };
    match expression(g, &c)? {
        Some(Expression::Function(call)) => Ok(Some(*call)),
        _ => Err(Error::Mapping(format!(
            "rml:condition {c} is not a function execution"
        ))),
    }
}

/// A constant shortcut such as `rml:predicate ex:p`.
fn constant(term: Term, position: Position) -> Result<TermMap> {
    check_constant(&term, position)?;
    Ok(TermMap::constant(term))
}

fn check_constant(term: &Term, position: Position) -> Result<()> {
    let ok = match term {
        Term::BlankNode(_) => false,
        Term::Literal(_) => matches!(position, Position::Object | Position::Language),
        Term::Iri(_) => position != Position::Language,
        Term::Triple(_) => false,
    };
    if ok {
        Ok(())
    } else {
        Err(Error::Mapping(format!(
            "constant {term} is not allowed in a {position:?} map"
        )))
    }
}

fn term_map(g: &Graph, node: &Term, position: Position) -> Result<TermMap> {
    if g.objects(node, &rml("parentTriplesMap")).next().is_some() {
        return Err(Error::Mapping(format!(
            "referencing object map used as a {position:?} map"
        )));
    }
    if g.one(node, "quotedTriplesMap")?.is_some() {
        return Err(Error::Mapping(format!(
            "rml:quotedTriplesMap is only allowed in subject and object maps, not in a {position:?} map"
        )));
    }
    let expression = expression(g, node)?;
    // A gather map's own expression only names the collection's head node.
    let gathering = g.one(node, "gather")?.is_some();
    if gathering && !matches!(position, Position::Subject | Position::Object) {
        return Err(Error::Mapping(format!(
            "rml:gather is not allowed in a {position:?} map"
        )));
    }

    let explicit = match g.one(node, "termType")? {
        None => None,
        Some(t) => Some(match t.as_iri().and_then(|i| i.strip_prefix(RML)) {
            Some("IRI") => TermType::Iri,
            Some("UnsafeIRI") => TermType::UnsafeIri,
            Some("URI") => TermType::Uri,
            Some("UnsafeURI") => TermType::UnsafeUri,
            Some("BlankNode") => TermType::BlankNode,
            Some("Literal") => TermType::Literal,
            _ => return Err(Error::Mapping(format!("unknown rml:termType {t}"))),
        }),
    };

    let mut datatype_map = None;
    let mut dts = Vec::new();
    for d in g.all(node, "datatypeMap") {
        dts.push(term_map(g, &d, Position::Datatype)?);
    }
    for d in g.all(node, "datatype") {
        dts.push(constant(d, Position::Datatype)?);
    }
    if dts.len() > 1 {
        return Err(Error::Mapping(format!(
            "{node} has more than one datatype map"
        )));
    }
    if let Some(d) = dts.pop() {
        datatype_map = Some(Box::new(d));
    }

    let mut language_map = None;
    let mut langs = Vec::new();
    for l in g.all(node, "languageMap") {
        langs.push(term_map(g, &l, Position::Language)?);
    }
    for l in g.all(node, "language") {
        if let Term::Literal(lit) = &l
            && !is_valid_language_tag(&lit.value)
        {
            return Err(Error::Mapping(format!(
                "invalid language tag {:?}",
                lit.value
            )));
        }
        langs.push(constant(l, Position::Language)?);
    }
    if langs.len() > 1 {
        return Err(Error::Mapping(format!(
            "{node} has more than one language map"
        )));
    }
    if let Some(l) = langs.pop() {
        language_map = Some(Box::new(l));
    }
    if datatype_map.is_some() && language_map.is_some() {
        return Err(Error::Mapping(format!(
            "{node} has both a datatype map and a language map"
        )));
    }

    let term_type = match (&expression, explicit) {
        // Constants are not considered as having a term type: the constant decides.
        (Some(Expression::Constant(c)), _) => {
            check_constant(c, position)?;
            TermMap::constant(c.clone()).term_type
        }
        (_, Some(t)) => t,
        (None, None) if gathering => TermType::BlankNode,
        (Some(_), None) if gathering => TermType::Iri,
        (Some(Expression::Reference(_) | Expression::Function(_)), None)
            if position == Position::Object =>
        {
            TermType::Literal
        }
        (_, None)
            if position == Position::Object
                && (datatype_map.is_some() || language_map.is_some()) =>
        {
            TermType::Literal
        }
        (_, None) if matches!(position, Position::Language) => TermType::Literal,
        (_, None) => TermType::Iri,
    };

    let allowed = match position {
        Position::Subject => !matches!(term_type, TermType::Literal),
        Position::Predicate | Position::Graph | Position::Datatype => {
            !matches!(term_type, TermType::Literal | TermType::BlankNode)
        }
        Position::Object => !(gathering && term_type == TermType::Literal),
        Position::Language => term_type == TermType::Literal,
    };
    if !allowed {
        return Err(Error::Mapping(format!(
            "term type {term_type:?} is not allowed in a {position:?} map"
        )));
    }
    if expression.is_none() && term_type != TermType::BlankNode {
        return Err(Error::Mapping(format!(
            "{node} has no rml:constant, rml:reference or rml:template"
        )));
    }
    if (datatype_map.is_some() || language_map.is_some()) && term_type != TermType::Literal {
        return Err(Error::Mapping(format!(
            "{node} has a datatype or language map but is not a literal term map"
        )));
    }

    // Triples whose object comes from a datatype or language map with a
    // target are exported there too, so the object map carries those targets.
    let mut targets = targets_of(g, node);
    for m in datatype_map.iter().chain(&language_map) {
        targets.extend(&m.targets);
    }
    targets.sort_unstable();
    targets.dedup();

    Ok(TermMap {
        expression,
        term_type,
        datatype_map,
        language_map,
        condition: condition(g, node)?,
        targets,
        gather: None,
        quoted: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turtle;

    fn parse(ttl: &str) -> Result<Mapping> {
        let prefixes =
            "@prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .\n";
        mapping_from_triples(turtle::parse(&format!("{prefixes}{ttl}"), None)?)
    }

    const LS: &str = r#"rml:logicalSource [ rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "s.json" ] ; rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" ]"#;

    #[test]
    fn star_maps() {
        let doc = |extra: &str, kinds: &str| {
            parse(&format!(
                r#"ex:A a {kinds} ; {LS} ; rml:subjectMap [ rml:template "http://e/{{$.id}}" ] ;
                     rml:predicateObjectMap [ rml:predicate ex:p ; rml:object ex:o ] .
                   ex:B a rml:TriplesMap ; {LS} ; {extra} ."#
            ))
        };
        let m = doc(
            "rml:subjectMap [ rml:quotedTriplesMap ex:A ] ; rml:predicateObjectMap [ rml:predicate ex:q ; rml:object ex:z ]",
            "rml:NonAssertedTriplesMap",
        )
        .unwrap();
        assert!(!m.triples_maps[0].asserted && m.triples_maps[1].asserted);
        let q = m.triples_maps[1]
            .subject_map
            .term_map
            .quoted
            .as_ref()
            .expect("a star map");
        assert_eq!(q.parent, 0);
        // A quoted triples map in a predicate map, a triples map of both kinds,
        // a star map with a template too.
        let pm = "rml:subjectMap [ rml:template \"http://e/b\" ] ; rml:predicateObjectMap [ rml:predicateMap [ rml:quotedTriplesMap ex:A ] ; rml:object ex:z ]";
        assert!(doc(pm, "rml:TriplesMap").is_err());
        let sm = "rml:subjectMap [ rml:quotedTriplesMap ex:A ] ; rml:predicateObjectMap [ rml:predicate ex:q ; rml:object ex:z ]";
        assert!(doc(sm, "rml:AssertedTriplesMap , rml:NonAssertedTriplesMap").is_err());
        let both = "rml:subjectMap [ rml:quotedTriplesMap ex:A ; rml:template \"http://e/b\" ]";
        assert!(doc(both, "rml:TriplesMap").is_err());
    }

    #[test]
    fn logical_view_structure() {
        let doc = |view: &str| {
            parse(&format!(
                r#"ex:src a rml:LogicalSource ; rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" ;
                     rml:source [ a rml:RelativePathSource ; rml:path "s.json" ] .
                   {view}
                   ex:TM a rml:TriplesMap ; rml:logicalSource ex:v ;
                     rml:subjectMap [ rml:template "http://e/{{a}}" ] ."#
            ))
        };
        let field = |name: &str| {
            format!(r#"rml:field [ rml:fieldName "{name}" ; rml:reference "$.{name}" ]"#)
        };
        let m = doc(&format!(
            "ex:v rml:viewOn ex:src ; {} ; {} .",
            field("a"),
            field("b")
        ))
        .unwrap();
        let Source::View(v) = &m.triples_maps[0].logical_source.source else {
            panic!("not a view");
        };
        assert_eq!(v.fields.len(), 2);
        // Two fields with one name, a view on itself, a view through a join on itself.
        assert!(
            doc(&format!(
                "ex:v rml:viewOn ex:src ; {} ; {} .",
                field("a"),
                field("a")
            ))
            .is_err()
        );
        assert!(doc(&format!("ex:v rml:viewOn ex:v ; {} .", field("a"))).is_err());
        let self_join = format!(
            r#"ex:v rml:viewOn ex:src ; {} ;
                 rml:leftJoin [ rml:parentLogicalView ex:v ; rml:joinCondition [ rml:child "a" ; rml:parent "a" ] ;
                                rml:field [ rml:fieldName "c" ; rml:reference "a" ] ] ."#,
            field("a")
        );
        assert!(doc(&self_join).is_err());
        // An unused view is still checked.
        let unused = format!("ex:w rml:viewOn ex:src ; {} ; {} .", field("a"), field("a"));
        assert!(
            doc(&format!(
                "ex:v rml:viewOn ex:src ; {} . {unused}",
                field("a")
            ))
            .is_err()
        );
    }

    #[test]
    fn gather_maps() {
        let om = |body: &str| {
            parse(&format!(
                r#"ex:TM a rml:TriplesMap ; {LS} ; rml:subjectMap [ rml:template "http://e/{{$.id}}" ] ;
                   rml:predicateObjectMap [ rml:predicate ex:p ; rml:objectMap [ {body} ] ] ."#
            ))
        };
        let list = "rml:gatherAs <http://www.w3.org/1999/02/22-rdf-syntax-ns#List>";
        let m = om(&format!(
            r#"rml:gather ( [ rml:reference "$.a" ] ) ; {list}"#
        ))
        .unwrap();
        let ObjectMap::Term(t) = &m.triples_maps[0].predicate_object_maps[0].object_maps[0] else {
            panic!("not a term map");
        };
        let g = t.gather.as_ref().expect("a gather map");
        assert_eq!(
            (g.gather_as, g.strategy, g.allow_empty),
            (Collection::List, Strategy::Append, false)
        );
        // Without an expression the head is a new blank node; with one, an IRI by default.
        assert_eq!(t.term_type, TermType::BlankNode);
        let named = om(&format!(
            r#"rml:reference "$.id" ; rml:gather ( [ rml:reference "$.a" ] ) ; {list}"#
        ));
        let ObjectMap::Term(t) =
            &named.unwrap().triples_maps[0].predicate_object_maps[0].object_maps[0]
        else {
            panic!("not a term map");
        };
        assert_eq!(t.term_type, TermType::Iri);
        // Structural errors.
        assert!(om(r#"rml:gather ( [ rml:reference "$.a" ] )"#).is_err()); // no rml:gatherAs
        assert!(om(&format!("rml:gather () ; {list}")).is_err());
        assert!(om(&format!(r#"rml:termType rml:Literal ; rml:reference "$.id" ; rml:gather ( [ rml:reference "$.a" ] ) ; {list}"#)).is_err());
        assert!(parse(&format!(
            r#"ex:TM a rml:TriplesMap ; {LS} ; rml:subjectMap [ rml:template "http://e/{{$.id}}" ] ;
               rml:predicateObjectMap [ rml:predicateMap [ rml:gather ( [ rml:reference "$.a" ] ) ; {list} ] ; rml:object "x" ] ."#
        ))
        .is_err());
    }

    #[test]
    fn defaults_and_shortcuts() {
        let m = parse(&format!(
            r#"ex:TM a rml:TriplesMap ; {LS} ;
               rml:subjectMap [ rml:template "http://e/{{$.id}}" ; rml:class ex:C ] ;
               rml:predicateObjectMap [ rml:predicate ex:p ; rml:objectMap [ rml:reference "$.n" ] ] ."#
        ))
        .unwrap();
        let tm = &m.triples_maps[0];
        assert_eq!(tm.subject_map.term_map.term_type, TermType::Iri);
        assert_eq!(tm.subject_map.classes, ["http://example.com/C"]);
        let ObjectMap::Term(om) = &tm.predicate_object_maps[0].object_maps[0] else {
            panic!()
        };
        assert_eq!(om.term_type, TermType::Literal);
        assert_eq!(
            tm.logical_source.source,
            Source::RelativePath {
                root: PathRoot::MappingDirectory,
                path: "s.json".into(),
                compression: Compression::None,
                encoding: SourceEncoding::Utf8,
            }
        );
    }

    #[test]
    fn xpath_reference_formulations() {
        let rf = |body: &str| {
            let m = parse(&format!(
                r#"ex:TM rml:logicalSource [ rml:source [ a rml:RelativePathSource ; rml:path "s.xml" ] ; rml:referenceFormulation {body} ; rml:iterator "/a" ] ;
                   rml:subjectMap [ rml:template "http://e/{{@id}}" ] ."#
            ))
            .unwrap();
            m.triples_maps[0]
                .logical_source
                .reference_formulation
                .clone()
        };
        assert_eq!(
            rf("rml:XPath"),
            Some(ReferenceFormulation::XPath { namespaces: vec![] })
        );
        assert_eq!(
            rf(
                r#"[ a rml:XPathReferenceFormulation ; rml:namespace [ a rml:Namespace ; rml:namespacePrefix "ex" ; rml:namespaceURL "http://example.org" ] ]"#
            ),
            Some(ReferenceFormulation::XPath {
                namespaces: vec![("ex".into(), "http://example.org".into())]
            })
        );
    }

    #[test]
    fn remote_sources() {
        let prefixes = "@prefix dcat: <http://www.w3.org/ns/dcat#> .
            @prefix td: <https://www.w3.org/2019/wot/td#> .
            @prefix hctl: <https://www.w3.org/2019/wot/hypermedia#> .
            @prefix htv: <http://www.w3.org/2011/http#> .";
        let source = |desc: &str| {
            parse(&format!(
                r#"{prefixes}
                   ex:TM rml:logicalSource [ rml:source [ a rml:Source ; {desc} ] ;
                       rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" ] ;
                     rml:subjectMap [ rml:template "http://e/{{$.id}}" ] ."#
            ))
            .map(|m| m.triples_maps[0].logical_source.source.clone())
        };
        // A DCAT download URL, which may lie under the rml: namespace.
        let url = "http://w3id.org/rml/resources/rml-io/RMLSTC0006a/Friends.json";
        assert_eq!(
            source(&format!("dcat:downloadURL <{url}>")).unwrap(),
            Source::Remote(RemoteSource {
                url: url.into(),
                content_type: None,
                headers: vec![],
            })
        );
        // A Web of Things form with content type, method and headers.
        let form = |target: &str, extra: &str| {
            format!(
                r#"td:hasPropertyAffordance [ td:hasForm [ hctl:hasTarget "{target}" ;
                     hctl:forContentType "application/json" {extra} ] ]"#
            )
        };
        assert_eq!(
            source(&form(
                "https://api.example.com/x",
                r#"; htv:methodName "GET" ; htv:headers ( [ htv:fieldName "User-Agent" ; htv:fieldValue "rmlrs" ] )"#
            ))
            .unwrap(),
            Source::Remote(RemoteSource {
                url: "https://api.example.com/x".into(),
                content_type: Some("application/json".into()),
                headers: vec![("User-Agent".into(), "rmlrs".into())],
            })
        );
        // Streams and other methods are not supported; a form without a
        // target is not valid.
        for unsupported in [
            form("kafka://localhost/topic", ""),
            form("mqtt://localhost/topic", ""),
            form("http://h/", r#"; htv:methodName "POST""#),
            r#"td:hasPropertyAffordance [ td:hasForm [ hctl:hasTarget "http://h/" ; hctl:forContentType "text/event-stream" ] ]"#.into(),
            "dcat:accessURL <http://h/service>".into(),
        ] {
            assert!(
                matches!(source(&unsupported), Err(Error::Unsupported(_))),
                "{unsupported}"
            );
        }
        assert!(matches!(
            source(
                "td:hasPropertyAffordance [ td:hasForm [ hctl:forContentType \"application/json\" ] ]"
            ),
            Err(Error::Mapping(_))
        ));
        assert!(matches!(
            source("dcat:downloadURL \"http://h/x.json\""),
            Err(Error::Mapping(_))
        ));

        // The URLs a mapping reads, once each, also under a logical view.
        let m = parse(&format!(
            r#"{prefixes}
               ex:a rml:source [ dcat:downloadURL <http://h/a.json> ] ; rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" .
               ex:b rml:source [ dcat:downloadURL <http://h/b.json> ] ; rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" .
               ex:v rml:viewOn ex:b ; rml:field [ rml:fieldName "id" ; rml:reference "$.id" ] .
               ex:T1 rml:logicalSource ex:a ; rml:subjectMap [ rml:template "http://e/{{$.id}}" ] .
               ex:T2 rml:logicalSource ex:a ; rml:subjectMap [ rml:template "http://f/{{$.id}}" ] .
               ex:T3 rml:logicalSource ex:v ; rml:subjectMap [ rml:template "http://g/{{id}}" ] ."#
        ))
        .unwrap();
        assert_eq!(m.remote_urls(), ["http://h/a.json", "http://h/b.json"]);
    }

    #[test]
    fn sparql_sources() {
        let ls = |source: &str, rf: &str| {
            parse(&format!(
                r#"@prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
                   @prefix formats: <http://www.w3.org/ns/formats/> .
                   ex:TM rml:logicalSource [ rml:source {source} ; rml:referenceFormulation {rf} ;
                       rml:iterator "SELECT ?x WHERE {{ ?x ?p ?o }}" ] ;
                     rml:subjectMap [ rml:reference "x" ] ."#
            ))
            .map(|m| m.triples_maps[0].logical_source.clone())
        };
        let service = "[ a rml:Source, sd:Service ; sd:endpoint <http://h/sparql> ; sd:supportedLanguage sd:SPARQL11Query ]";
        let l = ls(service, "formats:SPARQL_Results_CSV").unwrap();
        assert_eq!(l.source, Source::SparqlEndpoint("http://h/sparql".into()));
        assert_eq!(
            l.reference_formulation,
            Some(ReferenceFormulation::SparqlResults(
                SparqlResultsFormat::Csv
            ))
        );
        for (name, format) in [
            ("TSV", SparqlResultsFormat::Tsv),
            ("JSON", SparqlResultsFormat::Json),
            ("XML", SparqlResultsFormat::Xml),
        ] {
            let l = ls(service, &format!("formats:SPARQL_Results_{name}")).unwrap();
            assert_eq!(
                l.reference_formulation,
                Some(ReferenceFormulation::SparqlResults(format))
            );
        }
        // Another formats: IRI is no reference formulation RMLrs knows.
        assert!(matches!(
            ls(service, "formats:Turtle").unwrap().reference_formulation,
            Some(ReferenceFormulation::Other(_))
        ));
        assert!(matches!(
            ls(
                "[ sd:endpoint \"http://h/sparql\" ]",
                "formats:SPARQL_Results_CSV"
            ),
            Err(Error::Mapping(_))
        ));
    }

    #[test]
    fn structural_errors() {
        for bad in [
            format!("ex:TM {LS} ."),
            format!(
                r#"ex:TM {LS} ; rml:subjectMap [ rml:reference "$.a" ; rml:termType rml:Literal ] ."#
            ),
            format!(r#"ex:TM {LS} ; rml:subjectMap [ rml:constant "School" ] ."#),
            format!(
                r#"ex:TM {LS} ; rml:subject ex:a ; rml:subjectMap [ rml:template "x{{a}}" ] ."#
            ),
            format!(
                r#"ex:TM {LS} ; rml:subject ex:a ; rml:predicateObjectMap [ rml:predicate ex:p ; rml:object "o" ; rml:graphMap [ rml:reference "$.a" ; rml:termType rml:Literal ] ] ."#
            ),
        ] {
            assert!(matches!(parse(&bad), Err(Error::Mapping(_))), "{bad}");
        }
    }

    /// The source of a one-map mapping whose source node is `source`.
    fn source_of(source: &str) -> Result<Source> {
        let m = parse(&format!(
            r#"@prefix csvw: <http://www.w3.org/ns/csvw#> .
               ex:TM rml:logicalSource [ rml:source {source} ; rml:referenceFormulation rml:CSV ] ;
                 rml:subjectMap [ rml:template "http://e/{{a}}" ] ."#
        ))?;
        Ok(m.triples_maps[0].logical_source.source.clone())
    }

    fn dialect_of(dialect: &str) -> Result<CsvDialect> {
        match source_of(&format!(
            r#"[ a csvw:Table ; csvw:url "s.csv" ; csvw:dialect [ {dialect} ] ]"#
        ))? {
            Source::CsvwTable(t) => Ok(t.dialect),
            other => panic!("not a CSVW table: {other:?}"),
        }
    }

    #[test]
    fn source_encodings() {
        let encoding = |source: &str| match source_of(source).unwrap() {
            Source::RelativePath { encoding, .. } => encoding,
            Source::CsvwTable(t) => t.encoding,
            other => panic!("{other:?}"),
        };
        let file = |e: &str| format!(r#"[ rml:path "s.csv" {e} ]"#);
        assert_eq!(encoding(&file("")), SourceEncoding::Utf8);
        assert_eq!(
            encoding(&file("; rml:encoding rml:UTF-8")),
            SourceEncoding::Utf8
        );
        assert_eq!(
            encoding(&file("; rml:encoding rml:UTF-16")),
            SourceEncoding::Utf16Be
        );
        let table = |e: &str| {
            format!(r#"[ a csvw:Table ; csvw:url "s.csv" ; csvw:dialect [ csvw:encoding "{e}" ] ]"#)
        };
        assert_eq!(encoding(&table("UTF-8")), SourceEncoding::Utf8);
        assert_eq!(encoding(&table("utf-16")), SourceEncoding::Utf16Le);
        assert_eq!(encoding(&table("UTF-16BE")), SourceEncoding::Utf16Be);
        // rml:encoding takes precedence over csvw:encoding.
        assert_eq!(
            encoding(
                r#"[ a csvw:Table ; csvw:url "s.csv" ; rml:encoding rml:UTF-16 ;
                     csvw:dialect [ csvw:encoding "utf-8" ] ]"#
            ),
            SourceEncoding::Utf16Be
        );
        assert!(matches!(
            source_of(&table("iso-8859-1")),
            Err(Error::Unsupported(_))
        ));
        assert!(matches!(
            source_of(&file("; rml:encoding ex:UTF-32")),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn csvw_tables() {
        match source_of(
            r#"[ a rml:Source, csvw:Table ; csvw:url "d/s.csv"^^<http://www.w3.org/2001/XMLSchema#anyURI> ;
                 rml:compression rml:gzip ]"#,
        )
        .unwrap()
        {
            Source::CsvwTable(t) => {
                assert_eq!(t.url, "d/s.csv");
                assert_eq!(t.compression, Compression::Gzip);
                assert_eq!(t.dialect, CsvDialect::default());
            }
            other => panic!("{other:?}"),
        }
        // A table without a type is recognised by its csvw:url.
        assert!(matches!(
            source_of(r#"[ csvw:url "s.csv" ]"#),
            Ok(Source::CsvwTable(_))
        ));
        assert!(matches!(
            source_of(r#"[ a csvw:Table ]"#),
            Err(Error::Mapping(_))
        ));
        assert!(matches!(
            source_of(r#"[ a csvw:Table ; csvw:url "s.csv" ; csvw:tableSchema [ ] ]"#),
            Err(Error::Unsupported(_))
        ));
    }

    #[test]
    fn csvw_nulls() {
        let m = parse(
            r#"@prefix csvw: <http://www.w3.org/ns/csvw#> .
               ex:TM rml:logicalSource [ rml:source [ a csvw:Table ; csvw:url "s.csv" ; csvw:null "NULL", "-" ] ;
                   rml:referenceFormulation rml:CSV ] ;
                 rml:subjectMap [ rml:template "http://e/{a}" ] ."#,
        )
        .unwrap();
        let mut nulls = m.triples_maps[0].logical_source.null_values.clone();
        nulls.sort();
        assert_eq!(nulls, ["-", "NULL"]);
    }

    #[test]
    fn csvw_dialects() {
        let d = dialect_of(
            r##"csvw:delimiter "\t" ; csvw:quoteChar "'" ; csvw:doubleQuote false ;
               csvw:commentPrefix "#" ; csvw:skipRows 2 ; csvw:skipColumns 1 ;
               csvw:skipBlankRows true ; csvw:lineTerminators "\r\n""##,
        )
        .unwrap();
        assert_eq!(
            d,
            CsvDialect {
                delimiter: '\t',
                quote: '\'',
                double_quote: false,
                comment_prefix: Some("#".into()),
                header_rows: 1,
                skip_rows: 2,
                skip_columns: 1,
                trim: Trim::None,
            }
        );
        let header = |p: &str| dialect_of(p).unwrap().header_rows;
        assert_eq!(
            header(r#"csvw:header "1"^^<http://www.w3.org/2001/XMLSchema#boolean>"#),
            1
        );
        assert_eq!(header("csvw:header false"), 0);
        assert_eq!(header("csvw:header false ; csvw:headerRowCount 2"), 2);
        let trim = |p: &str| dialect_of(p).unwrap().trim;
        assert_eq!(trim("csvw:trim true"), Trim::Both);
        assert_eq!(trim(r#"csvw:trim "start""#), Trim::Start);
        assert_eq!(trim(r#"csvw:trim "end""#), Trim::End);
        assert_eq!(trim("csvw:skipInitialSpace true"), Trim::Start);
        assert_eq!(
            trim("csvw:skipInitialSpace true ; csvw:trim false"),
            Trim::None
        );
        for (bad, unsupported) in [
            (r#"csvw:delimiter ";;""#, true),
            (r#"csvw:lineTerminators "\r""#, true),
            ("csvw:quoteChar \";\" ; csvw:delimiter \";\"", false),
            (r#"csvw:trim "both""#, false),
            (r#"csvw:skipRows "-1""#, false),
            (r#"csvw:delimiter ";" , ",""#, false),
        ] {
            let r = dialect_of(bad);
            if unsupported {
                assert!(matches!(r, Err(Error::Unsupported(_))), "{bad}: {r:?}");
            } else {
                assert!(matches!(r, Err(Error::Mapping(_))), "{bad}: {r:?}");
            }
        }
    }

    #[test]
    fn named_graphs_need_a_target_that_holds_them() {
        let targets = |format: &str| {
            format!(
                r#"ex:T a rml:LogicalTarget ; rml:serialization formats:{format} ;
                     rml:target [ a rml:Target ; rml:path "out" ] .
                   ex:Q a rml:LogicalTarget ; rml:serialization formats:N-Quads ;
                     rml:target [ a rml:Target ; rml:path "out.nq" ] ."#
            )
        };
        let doc = |sm_extra: &str, pom_extra: &str, format: &str| {
            parse(&format!(
                r#"@prefix formats: <http://www.w3.org/ns/formats/> .
                   ex:A a rml:TriplesMap ; {LS} ;
                     rml:subjectMap [ rml:template "http://e/{{$.id}}" {sm_extra} ] ;
                     rml:predicateObjectMap [ rml:predicate ex:p ; rml:object ex:o {pom_extra} ] .
                   {}"#,
                targets(format)
            ))
        };
        let named = r#"; rml:graphMap [ rml:template "http://e/g/{$.id}" ]"#;
        let to_t = "; rml:logicalTarget ex:T";
        for format in ["N-Triples", "Turtle", "N3", "RDF_JSON", "RDF_XML"] {
            // A graph map of the subject map, the target on the subject map.
            let e = doc(&format!("{named} {to_t}"), "", format).unwrap_err();
            assert!(matches!(e, Error::Mapping(_)), "{format}: {e}");
            assert!(e.to_string().contains("named graphs"), "{format}: {e}");
            // A graph map of the predicate-object map, the target on its object.
            let pom = format!("{named} ; rml:objectMap [ rml:constant ex:o2 {to_t} ]");
            assert!(
                matches!(doc("", &pom, format), Err(Error::Mapping(_))),
                "{format}"
            );
            // ... and on its predicate.
            let pom = format!("{named} ; rml:predicateMap [ rml:constant ex:p2 {to_t} ]");
            assert!(
                matches!(doc("", &pom, format), Err(Error::Mapping(_))),
                "{format}"
            );
            // The target on a named graph's own graph map.
            let own = "; rml:graphMap [ rml:constant ex:g ; rml:logicalTarget ex:T ]";
            assert!(
                matches!(doc(own, "", format), Err(Error::Mapping(_))),
                "{format}"
            );
            // Class triples go to the subject map's targets in its graphs.
            let classes = parse(&format!(
                r#"@prefix formats: <http://www.w3.org/ns/formats/> .
                   ex:A a rml:TriplesMap ; {LS} ;
                     rml:subjectMap [ rml:template "http://e/{{$.id}}" ; rml:class ex:C {named} {to_t} ] .
                   {}"#,
                targets(format)
            ));
            assert!(matches!(classes, Err(Error::Mapping(_))), "{format}");

            // Allowed: the default graph, and targets the named graph does
            // not reach.
            let default = "; rml:graphMap [ rml:constant rml:defaultGraph ]";
            doc(&format!("{default} {to_t}"), "", format).unwrap();
            doc(to_t, "", format).unwrap();
            let split =
                "; rml:graphMap [ rml:constant rml:defaultGraph ; rml:logicalTarget ex:T ] ;
                           rml:graphMap [ rml:constant ex:g ; rml:logicalTarget ex:Q ]";
            doc(split, "", format).unwrap();
        }
        for format in ["N-Quads", "JSON-LD"] {
            doc(&format!("{named} {to_t}"), "", format).unwrap();
        }
    }
}
