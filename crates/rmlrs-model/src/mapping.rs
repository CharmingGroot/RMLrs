// SPDX-License-Identifier: Apache-2.0
//! The RML mapping model, after shortcut expansion and validation.
//!
//! The parser turns a mapping graph into these types; the engine only ever
//! sees these types, never the raw graph.

use crate::Term;

/// A whole mapping: the triples maps of one mapping document.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapping {
    pub triples_maps: Vec<TriplesMap>,
    /// Logical targets (RML-IO), deduplicated by effective equality. Term
    /// maps refer to them by index.
    pub logical_targets: Vec<LogicalTarget>,
    /// Written in a legacy vocabulary (R2RML or old RML). Old RML gave JSON
    /// values no natural datatype: they are read as plain strings, in the
    /// form the document has them.
    pub legacy: bool,
}

impl Mapping {
    /// The URLs of the remote documents the mapping reads
    /// (`dcat:downloadURL`, Web of Things targets, HTTP(S) `csvw:url`),
    /// each once, in mapping order, including those under logical views. A
    /// caller can fetch them itself and hand them over in memory, keyed by
    /// their URL.
    pub fn remote_urls(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for s in self.sources() {
            let url = match s {
                Source::Remote(r) => r.url.as_str(),
                Source::CsvwTable(t) if is_http_url(&t.url) => t.url.as_str(),
                _ => continue,
            };
            if !out.contains(&url) {
                out.push(url);
            }
        }
        out
    }

    /// The SPARQL endpoints the mapping queries, each once, in mapping order.
    pub fn sparql_endpoints(&self) -> Vec<&str> {
        let mut out = Vec::new();
        for s in self.sources() {
            if let Source::SparqlEndpoint(e) = s
                && !out.contains(&e.as_str())
            {
                out.push(e.as_str());
            }
        }
        out
    }

    /// Every source the triples maps read, also those under logical views.
    fn sources(&self) -> Vec<&Source> {
        fn visit<'a>(ls: &'a LogicalSource, out: &mut Vec<&'a Source>) {
            out.push(&ls.source);
            if let Source::View(v) = &ls.source {
                visit(&v.view_on, out);
                for j in &v.joins {
                    visit(&j.parent, out);
                }
            }
        }
        let mut out = Vec::new();
        for tm in &self.triples_maps {
            visit(&tm.logical_source, &mut out);
        }
        out
    }
}

/// Whether a URL is an `http:` or `https:` URL (scheme case-insensitive).
pub fn is_http_url(url: &str) -> bool {
    url.split_once(':')
        .is_some_and(|(s, _)| s.eq_ignore_ascii_case("http") || s.eq_ignore_ascii_case("https"))
}

/// Where triples go besides (or instead of) the processor's default output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicalTarget {
    pub root: PathRoot,
    pub path: String,
    pub serialization: Serialization,
    pub encoding: TextEncoding,
    pub compression: Compression,
}

/// `rml:compression` of a file source or target (RML-IO).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Compression {
    /// `rml:none`, and the default.
    #[default]
    None,
    /// `rml:gzip`: one GZip stream.
    Gzip,
    /// `rml:zip`: a Zip archive holding one file.
    Zip,
    /// `rml:targz`: a Tar archive holding one file, GZip-compressed.
    TarGz,
    /// `rml:tarxz`: a Tar archive holding one file, Xz-compressed.
    TarXz,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Serialization {
    NQuads,
    NTriples,
    Turtle,
    N3,
    RdfJson,
    JsonLd,
    RdfXml,
}

impl Serialization {
    /// Whether the format can hold named graphs. RML-IO (section 7.4) calls
    /// a mapping that sends a named graph to any other format invalid.
    pub fn has_named_graphs(self) -> bool {
        matches!(self, Serialization::NQuads | Serialization::JsonLd)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    Utf8,
    Utf16,
}

/// The character encoding of a file source: `rml:encoding`, or
/// `csvw:encoding` of a CSVW table. The decoded text is always UTF-8.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum SourceEncoding {
    #[default]
    Utf8,
    /// UTF-16, big-endian unless a byte order mark says otherwise.
    Utf16Be,
    /// UTF-16, little-endian unless a byte order mark says otherwise.
    Utf16Le,
}

/// A `csvw:Table` source: the CSV file at `url` and how to read it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CsvwTable {
    /// `csvw:url`. A relative URL is resolved against the mapping
    /// directory, as CSVW resolves it against the metadata document.
    pub url: String,
    pub compression: Compression,
    /// `rml:encoding`, else `csvw:encoding` of the dialect.
    pub encoding: SourceEncoding,
    pub dialect: CsvDialect,
}

/// How a CSV file is split into rows and cells: the `csvw:Dialect`
/// properties that RMLrs reads. The default is plain RFC 4180 with one
/// header row, as for any `rml:CSV` source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CsvDialect {
    /// `csvw:delimiter`.
    pub delimiter: char,
    /// `csvw:quoteChar`.
    pub quote: char,
    /// `csvw:doubleQuote`: a quote inside a quoted cell is written twice
    /// (`true`) or escaped with a backslash (`false`).
    pub double_quote: bool,
    /// `csvw:commentPrefix`: rows that start with it are skipped.
    pub comment_prefix: Option<String>,
    /// `csvw:headerRowCount` / `csvw:header`. The first header row names the
    /// columns; with none, columns are named `_col.1`, `_col.2`, ...
    pub header_rows: usize,
    /// `csvw:skipRows`: rows skipped before the header.
    pub skip_rows: usize,
    /// `csvw:skipColumns`: columns skipped at the start of every row.
    pub skip_columns: usize,
    /// `csvw:trim` / `csvw:skipInitialSpace`.
    pub trim: Trim,
}

impl Default for CsvDialect {
    fn default() -> Self {
        CsvDialect {
            delimiter: ',',
            quote: '"',
            double_quote: true,
            comment_prefix: None,
            header_rows: 1,
            skip_rows: 0,
            skip_columns: 0,
            trim: Trim::None,
        }
    }
}

/// Which whitespace is removed around cell values (`csvw:trim`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum Trim {
    #[default]
    None,
    Start,
    End,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TriplesMap {
    /// The node that names the triples map in the mapping graph.
    pub node: Term,
    pub logical_source: LogicalSource,
    pub subject_map: SubjectMap,
    pub predicate_object_maps: Vec<PredicateObjectMap>,
    /// `rml:baseIRI`, overriding the processor's base IRI for this map.
    pub base_iri: Option<String>,
    /// `false` for an `rml:NonAssertedTriplesMap` (RML-star): its triples
    /// are only generated as quoted triples.
    pub asserted: bool,
}

/// A logical source. Two logical sources are *effectively equal* when these
/// fields are equal; the node that names them does not matter.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicalSource {
    pub source: Source,
    pub reference_formulation: Option<ReferenceFormulation>,
    pub iterator: Option<String>,
    /// `rml:null` values of the source: data equal to one of them is NULL.
    pub null_values: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ReferenceFormulation {
    JsonPath,
    Csv,
    /// `rml:SQLTable` / `rml:SQL2008Table`: the iterator is a table name.
    SqlTable,
    /// `rml:SQLQuery` / `rml:SQL2008Query`: the iterator is a query.
    SqlQuery,
    /// `rml:XPath`, or an `rml:XPathReferenceFormulation` with namespace
    /// declarations (prefix, namespace IRI) for the expressions.
    XPath {
        namespaces: Vec<(String, String)>,
    },
    /// `formats:SPARQL_Results_CSV` (or `_TSV`, `_JSON`, `_XML`): the
    /// iterator is a SPARQL SELECT query, each solution is an iteration, and
    /// references name its variables.
    SparqlResults(SparqlResultsFormat),
    Other(String),
}

/// The result format a SPARQL endpoint is asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SparqlResultsFormat {
    Csv,
    Tsv,
    Json,
    Xml,
}

impl SparqlResultsFormat {
    /// The media type of the format, for the `Accept` header.
    pub fn media_type(self) -> &'static str {
        match self {
            SparqlResultsFormat::Csv => "text/csv",
            SparqlResultsFormat::Tsv => "text/tab-separated-values",
            SparqlResultsFormat::Json => "application/sparql-results+json",
            SparqlResultsFormat::Xml => "application/sparql-results+xml",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Source {
    /// `rml:RelativePathSource`: a path relative to `root`.
    RelativePath {
        root: PathRoot,
        path: String,
        compression: Compression,
        encoding: SourceEncoding,
    },
    /// A CSV file described as a `csvw:Table` (RML-IO-Registry, CSVW).
    CsvwTable(CsvwTable),
    /// A plain string as source: a path relative to the working directory
    /// (the legacy form, still accepted).
    Path(String),
    /// A relational database (`d2rq:Database`).
    Database(DatabaseSource),
    /// A document behind an HTTP(S) IRI: the `dcat:downloadURL` of a
    /// `dcat:Distribution`, or the form of a `td:Thing` (Web of Things) with
    /// an `http`/`https` target.
    Remote(RemoteSource),
    /// A SPARQL endpoint (`sd:Service` with `sd:endpoint`). The logical
    /// source's iterator is the query, sent with the SPARQL 1.1 Protocol.
    SparqlEndpoint(String),
    /// The database given to the processor: the source of an R2RML logical
    /// table, which the mapping does not describe.
    ProcessorDatabase,
    /// A logical view (RML-LV); its logical source has no reference
    /// formulation or iterator of its own.
    View(Box<LogicalView>),
    /// Anything else, kept so that the engine can report it as unsupported.
    Other(String),
}

/// A logical view (RML-LV): fields derived from another abstract logical
/// source, possibly extended by joins with other views.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LogicalView {
    /// `rml:viewOn`.
    pub view_on: LogicalSource,
    pub fields: Vec<Field>,
    pub joins: Vec<ViewJoin>,
}

/// A field of a logical view, with its nested fields.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Field {
    /// The declared name (`rml:fieldName`).
    pub name: String,
    pub kind: FieldKind,
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// `rml:ExpressionField`: its records are the values of the expression.
    Expression(Expression),
    /// `rml:IterableField`: its records are the iterations the iterator
    /// selects, read with the reference formulation (the parent's if absent).
    Iterable {
        reference_formulation: Option<ReferenceFormulation>,
        iterator: Option<String>,
    },
}

/// `rml:leftJoin` or `rml:innerJoin` of a logical view.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ViewJoin {
    /// An inner join drops the iterations without a match; a left join
    /// keeps them, with no values for the joined fields.
    pub inner: bool,
    /// `rml:parentLogicalView`, as a logical source over that view.
    pub parent: LogicalSource,
    pub conditions: Vec<JoinCondition>,
    /// Expression fields evaluated on the matching parent iterations.
    pub fields: Vec<Field>,
}

/// How to fetch a [`Source::Remote`] document.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RemoteSource {
    /// The `http:` or `https:` IRI of the document.
    pub url: String,
    /// `hctl:forContentType`, sent as the `Accept` header.
    pub content_type: Option<String>,
    /// `htv:headers` of a Web of Things form (field name, field value).
    pub headers: Vec<(String, String)>,
}

/// Connection details of a `d2rq:Database` source.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DatabaseSource {
    /// JDBC connection string, e.g. `jdbc:postgresql://host:5432/db`.
    pub dsn: String,
    pub driver: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathRoot {
    MappingDirectory,
    CurrentWorkingDirectory,
}

/// How a term map produces its value(s).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expression {
    Constant(Term),
    Reference(String),
    Template(Template),
    /// `rml:functionExecution` (RML-FNML): one output of a function call.
    Function(Box<FunctionCall>),
}

/// A function execution with the output to use (RML-FNML).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FunctionCall {
    /// The function IRI (`rml:function`).
    pub function: String,
    /// Parameter IRI (`rml:parameter`) and the expression giving its value.
    pub inputs: Vec<(String, Expression)>,
    /// The output IRI (`rml:return`); `None` means the function's first one.
    pub output: Option<String>,
}

/// A parsed `rml:template`: literal text and `{reference}` parts.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Template {
    pub parts: Vec<TemplatePart>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TemplatePart {
    Text(String),
    Reference(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TermType {
    Iri,
    UnsafeIri,
    Uri,
    UnsafeUri,
    BlankNode,
    Literal,
}

/// A term map with its term type already resolved (explicit or default).
#[derive(Debug, Clone, PartialEq)]
pub struct TermMap {
    /// `None` only for a blank node term map without an expression, which
    /// produces a fresh blank node per iteration.
    pub expression: Option<Expression>,
    pub term_type: TermType,
    pub datatype_map: Option<Box<TermMap>>,
    pub language_map: Option<Box<TermMap>>,
    /// `rml:condition` (RML-FNML): the term map produces nothing unless this
    /// call returns `true`.
    pub condition: Option<FunctionCall>,
    /// Logical targets of this term map, including those of its datatype
    /// and language maps (indices into [`Mapping::logical_targets`]).
    pub targets: Vec<usize>,
    /// `rml:gather` (RML-CC): the term map generates collections or
    /// containers, and its expression, if any, names their head nodes.
    pub gather: Option<Box<GatherMap>>,
    /// `rml:quotedTriplesMap` (RML-star): the term map generates the triples
    /// of that triples map as quoted triples, joined like a referencing
    /// object map when the logical sources differ.
    pub quoted: Option<Box<RefObjectMap>>,
}

/// The collection or container part of a gather map (RML-CC).
#[derive(Debug, Clone, PartialEq)]
pub struct GatherMap {
    /// `rml:gather`: the term maps whose terms become the members, in order.
    /// Referencing object maps and nested gather maps are allowed.
    pub items: Vec<ObjectMap>,
    /// `rml:gatherAs`.
    pub gather_as: Collection,
    /// `rml:strategy`; `rml:append` when absent.
    pub strategy: Strategy,
    /// `rml:allowEmptyListAndContainer`: an empty gather still yields
    /// `rdf:nil` or a container without members.
    pub allow_empty: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    List,
    Bag,
    Seq,
    Alt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Strategy {
    /// The terms of all the term maps, one after the other, in one collection.
    Append,
    /// One collection per combination of one term from each term map.
    CartesianProduct,
}

impl TermMap {
    pub fn constant(term: Term) -> Self {
        let term_type = match &term {
            Term::Literal(_) => TermType::Literal,
            Term::BlankNode(_) => TermType::BlankNode,
            // Constants come from the mapping document, which holds no quoted triples.
            Term::Iri(_) | Term::Triple(_) => TermType::Iri,
        };
        TermMap {
            expression: Some(Expression::Constant(term)),
            term_type,
            datatype_map: None,
            language_map: None,
            condition: None,
            targets: Vec::new(),
            gather: None,
            quoted: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SubjectMap {
    pub term_map: TermMap,
    pub classes: Vec<String>,
    pub graph_maps: Vec<TermMap>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PredicateObjectMap {
    pub predicate_maps: Vec<TermMap>,
    pub object_maps: Vec<ObjectMap>,
    pub graph_maps: Vec<TermMap>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ObjectMap {
    Term(TermMap),
    Ref(RefObjectMap),
}

/// A referencing object map: the object is the subject of another triples map.
#[derive(Debug, Clone, PartialEq)]
pub struct RefObjectMap {
    /// Index into [`Mapping::triples_maps`].
    pub parent: usize,
    pub join_conditions: Vec<JoinCondition>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct JoinCondition {
    pub child: Expression,
    pub parent: Expression,
}
