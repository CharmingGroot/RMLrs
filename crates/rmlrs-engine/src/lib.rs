// SPDX-License-Identifier: Apache-2.0
//! Mapping execution: the "generated RDF triples of a triples map" algorithm
//! of RML-Core, over the logical sources a mapping points at.

use std::collections::hash_map::{DefaultHasher, Entry};
use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::rc::Rc;

mod dedup;
#[cfg(feature = "fnml")]
pub mod functions;
mod views;

use rmlrs_model::{
    Collection, Compression, CsvDialect, DatabaseSource, Error, Expression, FunctionCall,
    GatherMap, Literal, LogicalSource, Mapping, ObjectMap, PathRoot, Quad, RDF, RDF_FIRST, RDF_NIL,
    RDF_REST, RDF_TYPE, RefObjectMap, ReferenceFormulation, RemoteSource, Result, Source,
    SourceEncoding, SparqlResultsFormat, Strategy, Template, TemplatePart, Term, TermMap, TermType,
    Triple, TriplesMap,
};
use rmlrs_source::{Format, Iteration, Iterations, JsonPath, NaturalValue, Reference};
use rmlrs_term as term;

const DEFAULT_GRAPH: &str = "http://w3id.org/rml/defaultGraph";

/// Where to find sources, and the base IRI for relative IRIs.
#[derive(Debug, Clone, Default)]
pub struct Options {
    pub base_iri: String,
    /// Directory of the mapping document (`rml:MappingDirectory`).
    pub mapping_dir: PathBuf,
    /// The current working directory (`rml:CurrentWorkingDirectory`).
    pub working_dir: PathBuf,
    /// Documents the caller already holds in memory, keyed by the path the
    /// mapping gives (`rml:path`, or `csvw:url`) or by the URL of a remote
    /// source (`dcat:downloadURL`, a Web of Things `hctl:hasTarget`). They
    /// are text already, so no `rml:encoding` applies. They take precedence
    /// over files and requests, so the same mapping runs on files, on data
    /// fetched by the caller, and in environments without a file system or
    /// network.
    pub in_memory: HashMap<String, String>,
    /// The database an R2RML mapping's logical tables are read from (R2RML
    /// gives the processor a database; the mapping does not describe it).
    pub database: Option<DatabaseSource>,
    /// Functions for RML-FNML besides the built-in ones; they take
    /// precedence over built-in functions with the same IRI.
    #[cfg(feature = "fnml")]
    pub functions: Vec<functions::Function>,
}

/// Runs a mapping and returns the output dataset without duplicates, in the
/// order the quads were generated.
pub fn run(mapping: &Mapping, options: &Options) -> Result<Vec<Quad>> {
    let mut out = Vec::new();
    run_to(mapping, options, &mut |q| {
        out.push(q);
        Ok(())
    })?;
    Ok(out)
}

/// Runs a mapping and hands each quad of the default output to `sink` as
/// soon as it is generated. Quads that go only to logical targets are not
/// passed; use [`run_to_destinations`] to receive those too.
///
/// Sources are read one iteration at a time (CSV row by row), and nothing is
/// kept per quad except a 128-bit fingerprint for duplicate removal, so
/// memory stays far below the size of the output.
pub fn run_to(
    mapping: &Mapping,
    options: &Options,
    sink: &mut dyn FnMut(Quad) -> Result<()>,
) -> Result<()> {
    run_to_destinations(mapping, options, &mut |q, d| match d {
        Destination::Default => sink(q),
        Destination::Target(_) => Ok(()),
    })
}

/// Where a quad goes: the processor's default output, or one of the
/// mapping's logical targets (index into `Mapping::logical_targets`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Destination {
    Default,
    Target(usize),
}

impl Destination {
    fn code(self) -> u32 {
        match self {
            Destination::Default => u32::MAX,
            Destination::Target(i) => i as u32,
        }
    }
}

/// Runs a mapping and hands each quad to `sink` once per destination. A
/// quad goes to every logical target of the term maps that produced it, or
/// to the default output when none of them has a target (RML-IO).
pub fn run_to_destinations(
    mapping: &Mapping,
    options: &Options,
    sink: &mut dyn FnMut(Quad, Destination) -> Result<()>,
) -> Result<()> {
    let mut e = Engine::new(mapping, options, sink)?;
    for group in source_groups(&mapping.triples_maps) {
        e.triples_maps(&group)?;
    }
    e.close_lists()
}

/// Triples maps grouped by logical source, in order of first appearance.
/// Effectively equal logical sources give the same iterations, so the maps
/// of a group are fed from one pass over the source instead of one each.
fn source_groups(triples_maps: &[TriplesMap]) -> Vec<Vec<usize>> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut by_source: HashMap<&LogicalSource, usize> = HashMap::new();
    for (i, tm) in triples_maps.iter().enumerate() {
        match by_source.entry(&tm.logical_source) {
            Entry::Occupied(g) => groups[*g.get()].push(i),
            Entry::Vacant(slot) => {
                slot.insert(groups.len());
                groups.push(vec![i]);
            }
        }
    }
    groups
}

struct Engine<'m, 's> {
    mapping: &'m Mapping,
    options: &'m Options,
    refs: HashMap<(Format, String), Reference>,
    /// Join indexes keyed by the address of the referencing object map.
    joins: HashMap<usize, Rc<JoinIndex>>,
    /// Named collections and containers still open for appending (RML-CC),
    /// by head node and graphs.
    open: HashMap<(Term, Vec<Option<Term>>), Open<'m>>,
    /// The keys of `open` in order of creation, for closing the lists.
    open_order: Vec<(Term, Vec<Option<Term>>)>,
    /// While generating quoted triples (RML-star), the triples a triples map
    /// generates are collected here instead of being output; one level per
    /// quoted triples map being run.
    capture: Vec<Vec<Triple>>,
    graph_ids: HashMap<String, usize>,
    fresh_blank: usize,
    seen: dedup::FingerprintSet,
    /// `rml:null` values of the logical source being iterated.
    nulls: &'m [String],
    /// Distinct XPath namespace declarations; `Format::Xml(i)` uses set `i`.
    namespace_sets: Vec<&'m [(String, String)]>,
    /// XML logical sources with a reference that may look outside the
    /// iterated element; the others are read a chunk of elements at a time.
    xml_whole: HashSet<&'m LogicalSource>,
    /// Function calls checked against their descriptions, keyed by the
    /// address of the call in the mapping.
    #[cfg(feature = "fnml")]
    calls: HashMap<usize, BoundCall>,
    sink: &'s mut dyn FnMut(Quad, Destination) -> Result<()>,
}

/// A generated graph term and the logical targets of the graph map that made it.
type GraphOut<'m> = (Option<Term>, &'m [usize]);

/// Parent subjects keyed by the values of the parent side of the join conditions.
type JoinIndex = HashMap<Vec<String>, Vec<Term>>;

/// A collection or container gathered from one iteration (RML-CC), not yet
/// written: its cells are written into the graphs of the triple it is part of.
#[derive(Clone)]
struct Gathered<'m> {
    map: &'m GatherMap,
    /// The head node given by the gather map's expression, if it has one.
    name: Option<Term>,
    members: Vec<Member<'m>>,
    targets: &'m [usize],
}

#[derive(Clone)]
enum Member<'m> {
    Term(Term),
    Nested(Gathered<'m>),
}

/// A named collection or container that later iterations may append to.
struct Open<'m> {
    /// For a list, the last cell, whose `rdf:rest` is written on closing; for
    /// a container, unused.
    last: Term,
    /// For a container, the number of its next member.
    next: usize,
    graphs: Vec<GraphOut<'m>>,
    targets: &'m [usize],
}

/// A function call checked against the function's description.
#[cfg(feature = "fnml")]
struct BoundCall {
    function: functions::Function,
    /// For each parameter of the function, the index of its input.
    order: Vec<usize>,
    /// Index of the output to use.
    output: usize,
}

/// What an expression evaluates to, before term generation.
enum Value_ {
    Constant(Term),
    Natural(NaturalValue),
    Text(String),
}

impl<'m, 's> Engine<'m, 's> {
    fn new(
        mapping: &'m Mapping,
        options: &'m Options,
        sink: &'s mut dyn FnMut(Quad, Destination) -> Result<()>,
    ) -> Result<Self> {
        let mut e = Engine {
            mapping,
            options,
            refs: HashMap::new(),
            joins: HashMap::new(),
            open: HashMap::new(),
            open_order: Vec::new(),
            capture: Vec::new(),
            graph_ids: HashMap::new(),
            fresh_blank: 0,
            seen: dedup::FingerprintSet::default(),
            nulls: &[],
            namespace_sets: Vec::new(),
            xml_whole: HashSet::new(),
            #[cfg(feature = "fnml")]
            calls: HashMap::new(),
            sink,
        };
        for tm in &mapping.triples_maps {
            if let Some(ReferenceFormulation::XPath { namespaces }) =
                &tm.logical_source.reference_formulation
                && !e.namespace_sets.contains(&namespaces.as_slice())
            {
                e.namespace_sets.push(namespaces);
            }
        }
        e.compile()?;
        Ok(e)
    }

    /// Checks the mapping before reading any data: reference formulations,
    /// every reference expression, and join rules.
    fn compile(&mut self) -> Result<()> {
        let mapping: &'m Mapping = self.mapping;
        check_quoting_cycles(mapping)?;
        for tm in &mapping.triples_maps {
            let format = self.format_of(&tm.logical_source)?;
            match (format, &tm.logical_source.iterator) {
                (Format::Json, Some(it)) => {
                    JsonPath::parse(it)?;
                }
                (Format::Xml(set), it) => {
                    rmlrs_source::xml::compile(
                        it.as_deref().unwrap_or("/"),
                        self.namespace_sets[set as usize],
                    )?;
                }
                _ => {}
            }
            let mut refs = Vec::new();
            collect_refs(&tm.subject_map.term_map, &mut refs);
            tm.subject_map
                .graph_maps
                .iter()
                .for_each(|g| collect_refs(g, &mut refs));
            for pom in &tm.predicate_object_maps {
                pom.predicate_maps
                    .iter()
                    .chain(&pom.graph_maps)
                    .for_each(|m| collect_refs(m, &mut refs));
            }
            for om in object_maps_of(tm) {
                match om {
                    ObjectMap::Term(m) => collect_refs(m, &mut refs),
                    ObjectMap::Ref(r) => {
                        self.compile_join(tm, r, &mut refs, "referencing object map")?
                    }
                }
            }
            for r in star_maps_of(tm) {
                self.compile_join(tm, r, &mut refs, "star map")?;
            }
            if matches!(format, Format::Xml(_))
                && !refs.iter().all(|r| rmlrs_source::xml::is_local(r))
            {
                self.xml_whole.insert(&tm.logical_source);
            }
            if let Source::View(v) = &tm.logical_source.source {
                self.compile_view(v)?;
                views::check_keys(&views::view_keys(v), &refs)?;
            }
            self.compile_refs(format, refs)?;
            self.compile_calls(tm)?;
        }
        Ok(())
    }

    /// Checks a join with a parent triples map (a referencing object map or
    /// a star map) and compiles its parent references; the child references
    /// are added to `refs`.
    fn compile_join(
        &mut self,
        tm: &'m TriplesMap,
        r: &'m RefObjectMap,
        refs: &mut Vec<String>,
        what: &str,
    ) -> Result<()> {
        let mapping: &'m Mapping = self.mapping;
        let parent = &mapping.triples_maps[r.parent];
        if r.join_conditions.is_empty() && parent.logical_source != tm.logical_source {
            return Err(Error::Mapping(format!(
                "{what} over a different logical source needs a join condition"
            )));
        }
        let parent_format = self.format_of(&parent.logical_source)?;
        let mut parent_refs = Vec::new();
        for jc in &r.join_conditions {
            expr_refs(&jc.child, refs);
            expr_refs(&jc.parent, &mut parent_refs);
        }
        if let Source::View(v) = &parent.logical_source.source {
            views::check_keys(&views::view_keys(v), &parent_refs)?;
        }
        if matches!(parent_format, Format::Xml(_))
            && !parent_refs.iter().all(|r| rmlrs_source::xml::is_local(r))
        {
            self.xml_whole.insert(&parent.logical_source);
        }
        self.compile_refs(parent_format, parent_refs)
    }

    /// Checks every function call of a triples map against the function's
    /// description (RML-FNML).
    fn compile_calls(&mut self, tm: &'m TriplesMap) -> Result<()> {
        let mut calls = Vec::new();
        term_map_calls(&tm.subject_map.term_map, &mut calls);
        for m in &tm.subject_map.graph_maps {
            term_map_calls(m, &mut calls);
        }
        for pom in &tm.predicate_object_maps {
            for m in pom.predicate_maps.iter().chain(&pom.graph_maps) {
                term_map_calls(m, &mut calls);
            }
        }
        {
            for om in object_maps_of(tm) {
                match om {
                    ObjectMap::Term(m) => term_map_calls(m, &mut calls),
                    ObjectMap::Ref(r) => {
                        for jc in &r.join_conditions {
                            expr_calls(&jc.child, &mut calls);
                            expr_calls(&jc.parent, &mut calls);
                        }
                    }
                }
            }
        }
        #[cfg(feature = "fnml")]
        for call in calls {
            let function =
                *functions::find(&self.options.functions, &call.function).ok_or_else(|| {
                    Error::Mapping(format!("no implementation of function <{}>", call.function))
                })?;
            let (order, output) = functions::bind(&function, &call.inputs, call.output.as_deref())?;
            self.calls.insert(
                call_key(call),
                BoundCall {
                    function,
                    order,
                    output,
                },
            );
        }
        #[cfg(not(feature = "fnml"))]
        if !calls.is_empty() {
            return Err(Error::Unsupported(
                "RML-FNML function execution (Cargo feature \"fnml\")".into(),
            ));
        }
        Ok(())
    }

    /// Runs a function call for one iteration: once per combination of
    /// input values (multi-valued inputs give a Cartesian product).
    #[cfg(feature = "fnml")]
    fn call(&self, call: &FunctionCall, it: &Iteration) -> Result<Vec<Value_>> {
        let bound = &self.calls[&call_key(call)];
        let mut columns = Vec::with_capacity(bound.order.len());
        for &i in &bound.order {
            let values = self.lexicals(&call.inputs[i].1, it)?;
            if values.is_empty() {
                return Ok(Vec::new());
            }
            columns.push(values);
        }
        let mut out = Vec::new();
        for args in cartesian(&columns) {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            let mut outputs = (bound.function.run)(&args)?;
            if let Some(Some(v)) = outputs.get_mut(bound.output).map(Option::take) {
                out.push(Value_::Natural(v));
            }
        }
        Ok(out)
    }

    #[cfg(not(feature = "fnml"))]
    fn call(&self, _: &FunctionCall, _: &Iteration) -> Result<Vec<Value_>> {
        unreachable!("rejected by compile_calls")
    }

    fn format_of(&self, ls: &LogicalSource) -> Result<Format> {
        if let Source::View(_) = ls.source {
            return Ok(Format::View);
        }
        // SPARQL solutions are read like the rows of a view: one value or
        // none per variable.
        match (&ls.source, &ls.reference_formulation) {
            (Source::SparqlEndpoint(_), Some(ReferenceFormulation::SparqlResults(f))) => {
                return match f {
                    SparqlResultsFormat::Xml => {
                        Err(Error::Unsupported("SPARQL XML results".into()))
                    }
                    _ => Ok(Format::View),
                };
            }
            (Source::SparqlEndpoint(_), Some(ReferenceFormulation::Other(r))) => {
                return Err(Error::Unsupported(format!(
                    "reference formulation {r} for a SPARQL endpoint"
                )));
            }
            (Source::SparqlEndpoint(e), _) => {
                return Err(Error::Mapping(format!(
                    "the SPARQL endpoint {e} needs a formats:SPARQL_Results_* reference formulation"
                )));
            }
            (_, Some(ReferenceFormulation::SparqlResults(_))) => {
                return Err(Error::Unsupported(
                    "a SPARQL query over an RDF document (RMLrs does not include a SPARQL engine)"
                        .into(),
                ));
            }
            _ => {}
        }
        match &ls.reference_formulation {
            Some(ReferenceFormulation::JsonPath) => Ok(Format::Json),
            Some(ReferenceFormulation::Csv) => Ok(Format::Csv),
            Some(ReferenceFormulation::SqlTable | ReferenceFormulation::SqlQuery) => {
                Ok(Format::Sql)
            }
            Some(ReferenceFormulation::XPath { namespaces }) => {
                let set = self
                    .namespace_sets
                    .iter()
                    .position(|s| *s == namespaces.as_slice())
                    .expect("namespace sets are collected up front");
                Ok(Format::Xml(set as u32))
            }
            None => Err(Error::Mapping(
                "logical source without rml:referenceFormulation".into(),
            )),
            Some(ReferenceFormulation::Other(r)) => {
                Err(Error::Unsupported(format!("reference formulation {r}")))
            }
            Some(ReferenceFormulation::SparqlResults(_)) => unreachable!("handled above"),
        }
    }

    fn compile_refs(&mut self, format: Format, refs: Vec<String>) -> Result<()> {
        for r in refs {
            if let Entry::Vacant(slot) = self.refs.entry((format, r)) {
                let namespaces = match format {
                    Format::Xml(set) => self.namespace_sets[set as usize],
                    _ => &[],
                };
                let compiled = format.compile(&slot.key().1, namespaces)?;
                slot.insert(compiled);
            }
        }
        Ok(())
    }

    fn values(&self, it: &Iteration, reference: &str) -> Result<Vec<NaturalValue>> {
        let format = match it {
            Iteration::Json(_) => Format::Json,
            Iteration::Csv(_) => Format::Csv,
            Iteration::Sql(_) => Format::Sql,
            Iteration::Xml(n) => Format::Xml(n.namespace_set()),
            Iteration::View(_) => Format::View,
        };
        let compiled = &self.refs[&(format, reference.to_owned())];
        let mut values = match (it, compiled) {
            (Iteration::Json(v), Reference::Json(p)) if self.mapping.legacy => {
                rmlrs_source::evaluate_plain(v, p)?
            }
            _ => rmlrs_source::evaluate_reference(it, compiled)?,
        };
        if !self.nulls.is_empty() {
            values.retain(|v| !self.nulls.contains(&v.lexical));
        }
        Ok(values)
    }

    /// Opens a logical source for one pass over its iterations.
    fn open(&mut self, ls: &'m LogicalSource) -> Result<Iterations<'m>> {
        if let Source::View(view) = &ls.source {
            let rows = self.view_rows(view)?;
            return Ok(Box::new(rows.into_iter().map(|r| Ok(Iteration::View(r)))));
        }
        let database = match &ls.source {
            Source::Database(db) => Some(db),
            Source::ProcessorDatabase => Some(self.options.database.as_ref().ok_or_else(|| {
                Error::Mapping(
                    "an R2RML logical table needs a database connection (Options::database, --db-url)"
                        .into(),
                )
            })?),
            _ => None,
        };
        if let Some(db) = database {
            let iterator = ls
                .iterator
                .as_deref()
                .ok_or_else(|| Error::Mapping("SQL logical source without rml:iterator".into()))?;
            let is_table = ls.reference_formulation == Some(ReferenceFormulation::SqlTable);
            return rmlrs_source::sql::open(db, &rmlrs_source::sql::query_for(iterator, is_table));
        }
        let options: &'m Options = self.options;
        let format = self.format_of(ls)?;
        if matches!(ls.source, Source::CsvwTable(_)) && format != Format::Csv {
            return Err(Error::Mapping(
                "a csvw:Table source is CSV: its logical source needs rml:CSV".into(),
            ));
        }
        let namespaces = match format {
            Format::Xml(set) => self.namespace_sets[set as usize],
            _ => &[],
        };
        let iterator = ls.iterator.as_deref();
        let (key, path, compression, encoding, dialect) = match &ls.source {
            Source::RelativePath {
                root,
                path,
                compression,
                encoding,
            } => {
                let dir = match root {
                    PathRoot::MappingDirectory => &options.mapping_dir,
                    PathRoot::CurrentWorkingDirectory => &options.working_dir,
                };
                (path, Ok(dir.join(path)), *compression, *encoding, None)
            }
            Source::Path(path) => (
                path,
                Ok(options.working_dir.join(path)),
                Compression::None,
                SourceEncoding::Utf8,
                None,
            ),
            // A CSVW table on the web, unless the caller holds it in memory.
            Source::CsvwTable(table)
                if rmlrs_model::is_http_url(&table.url)
                    && !options.in_memory.contains_key(&table.url) =>
            {
                if table.compression != Compression::None {
                    return Err(Error::Unsupported(format!(
                        "rml:compression of a remote csvw:url <{}>",
                        table.url
                    )));
                }
                let remote = RemoteSource {
                    url: table.url.clone(),
                    content_type: Some("text/csv".into()),
                    headers: Vec::new(),
                };
                let reader = std::io::BufReader::new(fetch(&remote)?);
                let reader = rmlrs_source::encoding::decode(Box::new(reader), table.encoding);
                let url = table.url.as_str();
                let its = csv_rows(reader, &table.dialect).map_err(|e| with_path(e, url))?;
                return Ok(Box::new(its.map(move |r| r.map_err(|e| with_path(e, url)))));
            }
            Source::CsvwTable(table) => (
                &table.url,
                csvw_path(&options.mapping_dir, &table.url),
                table.compression,
                table.encoding,
                Some(&table.dialect),
            ),
            Source::Remote(remote) => {
                // A document the caller already holds, keyed by its URL,
                // takes the place of the request.
                if let Some(text) = options.in_memory.get(&remote.url) {
                    return format.open(Box::new(text.as_bytes()), iterator, namespaces);
                }
                let reader = std::io::BufReader::new(fetch(remote)?);
                let url = remote.url.as_str();
                let its = format
                    .open(Box::new(reader), iterator, namespaces)
                    .map_err(|e| with_path(e, url))?;
                return Ok(Box::new(its.map(move |r| r.map_err(|e| with_path(e, url)))));
            }
            Source::SparqlEndpoint(endpoint) => {
                let Some(ReferenceFormulation::SparqlResults(results)) = ls.reference_formulation
                else {
                    unreachable!("checked by format_of")
                };
                let query = iterator.ok_or_else(|| {
                    Error::Mapping(format!(
                        "the SPARQL endpoint {endpoint} needs a query (rml:iterator)"
                    ))
                })?;
                let reader = std::io::BufReader::new(sparql_query(endpoint, query, results)?);
                let its = rmlrs_source::sparql::solutions(results, Box::new(reader))
                    .map_err(|e| with_path(e, endpoint))?;
                return Ok(Box::new(
                    its.map(move |r| r.map_err(|e| with_path(e, endpoint))),
                ));
            }
            Source::Other(desc) => return Err(Error::Unsupported(format!("source type {desc}"))),
            Source::Database(_) | Source::ProcessorDatabase | Source::View(_) => {
                unreachable!("handled above")
            }
        };
        // In-memory documents are already decompressed and decoded text.
        if let Some(text) = options.in_memory.get(key) {
            if let Some(dialect) = dialect {
                return csv_rows(Box::new(text.as_bytes()), dialect);
            }
            return format.open(Box::new(text.as_bytes()), iterator, namespaces);
        }
        let path = path?;
        let file =
            File::open(&path).map_err(|e| Error::Source(format!("{}: {e}", path.display())))?;
        let shown = path.display().to_string();
        let reader =
            rmlrs_source::compression::open(file, compression).map_err(|e| with_path(e, &shown))?;
        let reader = rmlrs_source::encoding::decode(reader, encoding);
        let its: Iterations<'_> = match format {
            Format::Xml(set) if !self.xml_whole.contains(ls) => Box::new(
                rmlrs_source::xml::local_iterations(reader, iterator, namespaces, set)
                    .map_err(|e| with_path(e, &shown))?
                    .map(|r| r.map(Iteration::Xml)),
            ),
            _ => match dialect {
                Some(dialect) => csv_rows(reader, dialect).map_err(|e| with_path(e, &shown))?,
                None => format
                    .open(reader, iterator, namespaces)
                    .map_err(|e| with_path(e, &shown))?,
            },
        };
        Ok(Box::new(
            its.map(move |r| r.map_err(|e| with_path(e, &shown))),
        ))
    }

    /// Runs triples maps that share one logical source: each iteration is
    /// read once and handed to every map, in mapping order.
    fn triples_maps(&mut self, group: &[usize]) -> Result<()> {
        let mapping: &'m Mapping = self.mapping;
        let triples_maps = &mapping.triples_maps;
        let ls = &triples_maps[group[0]].logical_source;
        if matches!(ls.source, Source::Database(_) | Source::ProcessorDatabase) {
            self.index_parents(group)?;
        }
        let iterations = self.open(ls)?;
        let bases: Vec<String> = group
            .iter()
            .map(|&i| {
                triples_maps[i]
                    .base_iri
                    .as_deref()
                    .unwrap_or(&self.options.base_iri)
                    .to_owned()
            })
            .collect();
        self.nulls = &ls.null_values;

        for it in iterations {
            let it = &it?;
            for (&index, base) in group.iter().zip(&bases) {
                // Non-asserted triples maps only make quoted triples (RML-star).
                if triples_maps[index].asserted {
                    self.triples_map(index, it, base)?;
                }
            }
        }
        Ok(())
    }

    /// The triples one triples map generates from one iteration.
    fn triples_map(&mut self, index: usize, it: &Iteration, base: &str) -> Result<()> {
        let tm = &self.mapping.triples_maps[index];
        let sm = &tm.subject_map;
        let sm_t: &'m [usize] = &sm.term_map.targets;
        let subjects = match &sm.term_map.gather {
            None => self.terms(&sm.term_map, it, base)?,
            // The cells are written once, into the subject's graphs.
            Some(gm) => {
                let subject_graphs = self.graphs(&sm.graph_maps, it, base)?;
                let graphs = target_graphs(&sm.graph_maps, &[], &subject_graphs, &[]);
                let mut heads = Vec::new();
                for c in self.gather(tm, &sm.term_map, gm, it, base)? {
                    heads.push(self.write_gathered(&c, &graphs)?);
                }
                heads
            }
        };
        if subjects.is_empty() {
            return Ok(());
        }
        let subject_graphs = self.graphs(&sm.graph_maps, it, base)?;

        for s in &subjects {
            for class in &sm.classes {
                let graphs = target_graphs(&sm.graph_maps, &[], &subject_graphs, &[]);
                self.emit(
                    s,
                    &Term::iri(RDF_TYPE),
                    &Term::iri(class.as_str()),
                    &graphs,
                    [sm_t, &[], &[]],
                )?;
            }
        }

        for pom in &tm.predicate_object_maps {
            let mut predicates: Vec<(Term, &'m [usize])> = Vec::new();
            for pm in &pom.predicate_maps {
                predicates.extend(
                    self.terms(pm, it, base)?
                        .into_iter()
                        .map(|t| (t, pm.targets.as_slice())),
                );
            }
            if predicates.is_empty() {
                continue;
            }
            let pom_graphs = self.graphs(&pom.graph_maps, it, base)?;
            let graphs = target_graphs(
                &sm.graph_maps,
                &pom.graph_maps,
                &subject_graphs,
                &pom_graphs,
            );

            let mut objects: Vec<(Term, &'m [usize])> = Vec::new();
            for om in &pom.object_maps {
                match om {
                    // Collections are written into the graphs of the triples
                    // they are the object of.
                    ObjectMap::Term(m) if m.gather.is_some() => {
                        let gm = m.gather.as_deref().expect("checked");
                        for c in self.gather(tm, m, gm, it, base)? {
                            let head = self.write_gathered(&c, &graphs)?;
                            objects.push((head, m.targets.as_slice()));
                        }
                    }
                    ObjectMap::Term(m) => objects.extend(
                        self.terms(m, it, base)?
                            .into_iter()
                            .map(|t| (t, m.targets.as_slice())),
                    ),
                    // The object is the parent's subject; only this triples
                    // map's term maps decide the targets.
                    ObjectMap::Ref(r) => objects.extend(
                        self.ref_objects(tm, r, it)?
                            .into_iter()
                            .map(|t| (t, &[][..])),
                    ),
                }
            }
            for s in &subjects {
                for (p, p_t) in &predicates {
                    for (o, o_t) in &objects {
                        self.emit(s, p, o, &graphs, [sm_t, p_t, o_t])?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The collections or containers a gather map yields for one iteration
    /// (RML-CC): one per head node name and, with `rml:cartesianProduct`,
    /// per combination of member terms.
    fn gather(
        &mut self,
        tm: &'m TriplesMap,
        m: &'m TermMap,
        gm: &'m GatherMap,
        it: &Iteration,
        base: &str,
    ) -> Result<Vec<Gathered<'m>>> {
        // The expression (if any) names the head nodes; without one, each
        // collection gets a new blank node when it is written.
        let names: Vec<Option<Term>> = match &m.expression {
            None => {
                if self.terms(m, it, base)?.is_empty() {
                    return Ok(Vec::new()); // a false rml:condition
                }
                vec![None]
            }
            Some(_) => self.terms(m, it, base)?.into_iter().map(Some).collect(),
        };
        if names.is_empty() {
            return Ok(Vec::new());
        }
        let mut sets: Vec<Vec<Member<'m>>> = Vec::with_capacity(gm.items.len());
        for item in &gm.items {
            sets.push(match item {
                ObjectMap::Term(t) => match &t.gather {
                    Some(nested) => self
                        .gather(tm, t, nested, it, base)?
                        .into_iter()
                        .map(Member::Nested)
                        .collect(),
                    None => self
                        .terms(t, it, base)?
                        .into_iter()
                        .map(Member::Term)
                        .collect(),
                },
                ObjectMap::Ref(r) => self
                    .ref_objects(tm, r, it)?
                    .into_iter()
                    .map(Member::Term)
                    .collect(),
            });
        }
        let member_lists: Vec<Vec<Member<'m>>> = match gm.strategy {
            Strategy::Append => vec![sets.into_iter().flatten().collect()],
            Strategy::CartesianProduct => cartesian(&sets),
        };
        let mut out = Vec::new();
        for name in &names {
            for members in &member_lists {
                if members.is_empty() && !gm.allow_empty {
                    continue;
                }
                out.push(Gathered {
                    map: gm,
                    name: name.clone(),
                    members: members.clone(),
                    targets: &m.targets,
                });
            }
        }
        Ok(out)
    }

    /// Writes a gathered collection or container into `graphs` and returns
    /// its head node. A named one is appended to what earlier iterations
    /// wrote under the same name in the same graphs, so that it stays one
    /// well-formed collection or container; lists are closed at the end of
    /// the run ([`Self::close_lists`]).
    fn write_gathered(&mut self, c: &Gathered<'m>, graphs: &[GraphOut<'m>]) -> Result<Term> {
        let mut members = Vec::with_capacity(c.members.len());
        for m in &c.members {
            members.push(match m {
                Member::Term(t) => t.clone(),
                Member::Nested(n) => self.write_gathered(n, graphs)?,
            });
        }
        let targets = [c.targets, &[][..], &[][..]];
        let key = c.name.as_ref().map(|n| {
            (
                n.clone(),
                graphs.iter().map(|g| g.0.clone()).collect::<Vec<_>>(),
            )
        });
        let open = key.as_ref().and_then(|k| self.open.get(k));
        let (first, rest) = (Term::iri(RDF_FIRST), Term::iri(RDF_REST));
        match c.map.gather_as {
            Collection::List => {
                if members.is_empty() {
                    return Ok(Term::iri(RDF_NIL));
                }
                // The first cell is the head node, or follows the last cell
                // of the list written so far under the same name.
                let mut cell = match (&c.name, open) {
                    (Some(_), Some(o)) => {
                        let last = o.last.clone();
                        let cell = self.fresh_cell();
                        self.emit(&last, &rest, &cell, graphs, targets)?;
                        cell
                    }
                    (Some(name), None) => name.clone(),
                    (None, _) => self.fresh_cell(),
                };
                let head = c.name.clone().unwrap_or_else(|| cell.clone());
                for (i, m) in members.iter().enumerate() {
                    self.emit(&cell, &first, m, graphs, targets)?;
                    if i + 1 < members.len() {
                        let next = self.fresh_cell();
                        self.emit(&cell, &rest, &next, graphs, targets)?;
                        cell = next;
                    }
                }
                match key {
                    Some(k) => self.keep_open(k, cell, 0, graphs, c.targets),
                    None => self.emit(&cell, &rest, &Term::iri(RDF_NIL), graphs, targets)?,
                }
                Ok(head)
            }
            Collection::Bag | Collection::Seq | Collection::Alt => {
                let start = open.map_or(1, |o| o.next);
                let head = c.name.clone().unwrap_or_else(|| self.fresh_cell());
                let class = match c.map.gather_as {
                    Collection::Bag => "Bag",
                    Collection::Seq => "Seq",
                    _ => "Alt",
                };
                self.emit(
                    &head,
                    &Term::iri(RDF_TYPE),
                    &Term::Iri(format!("{RDF}{class}")),
                    graphs,
                    targets,
                )?;
                for (i, m) in members.iter().enumerate() {
                    let p = Term::Iri(format!("{RDF}_{}", start + i));
                    self.emit(&head, &p, m, graphs, targets)?;
                }
                if let Some(k) = key {
                    self.keep_open(k, head.clone(), start + members.len(), graphs, c.targets);
                }
                Ok(head)
            }
        }
    }

    fn keep_open(
        &mut self,
        key: (Term, Vec<Option<Term>>),
        last: Term,
        next: usize,
        graphs: &[GraphOut<'m>],
        targets: &'m [usize],
    ) {
        let open = Open {
            last,
            next,
            graphs: graphs.to_vec(),
            targets,
        };
        if self.open.insert(key.clone(), open).is_none() {
            self.open_order.push(key);
        }
    }

    /// Ends every named list with `rdf:nil`, once no iteration can append.
    fn close_lists(&mut self) -> Result<()> {
        let (rest, nil) = (Term::iri(RDF_REST), Term::iri(RDF_NIL));
        for key in std::mem::take(&mut self.open_order) {
            let Some(o) = self.open.remove(&key) else {
                continue;
            };
            // Containers have no end marker; their `last` is the head node.
            if o.next == 0 {
                self.emit(&o.last, &rest, &nil, &o.graphs, [o.targets, &[], &[]])?;
            }
        }
        Ok(())
    }

    fn fresh_cell(&mut self) -> Term {
        self.fresh_blank += 1;
        Term::BlankNode(format!("c{}", self.fresh_blank))
    }

    /// The parent subjects joined with one child iteration.
    fn ref_objects(
        &mut self,
        tm: &TriplesMap,
        r: &RefObjectMap,
        it: &Iteration,
    ) -> Result<Vec<Term>> {
        debug_assert!(
            !r.join_conditions.is_empty()
                || self.mapping.triples_maps[r.parent].logical_source == tm.logical_source
        );
        self.joined(r, it, false)
    }

    /// What a parent triples map gives one child iteration: its subjects
    /// (a referencing object map) or, with `quoted`, its triples as quoted
    /// triples (a star map, RML-star). Without join conditions the parent
    /// reads the same iteration; with them, the matching parent iterations.
    fn joined(&mut self, r: &RefObjectMap, it: &Iteration, quoted: bool) -> Result<Vec<Term>> {
        if r.join_conditions.is_empty() {
            return self.parent_terms(r.parent, it, quoted);
        }
        let key = join_key(r);
        let index = match self.joins.get(&key) {
            Some(i) => i.clone(),
            None => {
                let i = Rc::new(self.build_join(r, quoted)?);
                self.joins.insert(key, i.clone());
                i
            }
        };
        let mut child_sides = Vec::new();
        for jc in &r.join_conditions {
            child_sides.push(self.lexicals(&jc.child, it)?);
        }
        let mut out = Vec::new();
        for k in cartesian(&child_sides) {
            if let Some(subjects) = index.get(&k) {
                out.extend(subjects.iter().cloned());
            }
        }
        Ok(out)
    }

    /// Builds the join indexes of a group's triples maps before their logical
    /// source is opened. Built on the first child row instead, a large parent
    /// would leave a database child unread for as long as indexing takes, and
    /// the server may close the connection meanwhile (MySQL's
    /// `net_write_timeout`, 60 s by default; PostgreSQL's
    /// `idle_in_transaction_session_timeout` when set).
    fn index_parents(&mut self, group: &[usize]) -> Result<()> {
        let mapping: &'m Mapping = self.mapping;
        for &index in group {
            for om in object_maps_of(&mapping.triples_maps[index]) {
                let ObjectMap::Ref(r) = om else { continue };
                if !r.join_conditions.is_empty() && !self.joins.contains_key(&join_key(r)) {
                    let i = Rc::new(self.build_join(r, false)?);
                    self.joins.insert(join_key(r), i);
                }
            }
        }
        Ok(())
    }

    fn build_join(&mut self, r: &RefObjectMap, quoted: bool) -> Result<JoinIndex> {
        let mapping: &'m Mapping = self.mapping;
        let parent = &mapping.triples_maps[r.parent];
        let iterations = self.open(&parent.logical_source)?;
        let child_nulls = std::mem::replace(&mut self.nulls, &parent.logical_source.null_values);
        let index = self.index_parent(r, iterations, quoted);
        self.nulls = child_nulls;
        index
    }

    /// The subjects of a triples map for one iteration, or its triples as
    /// quoted triples.
    fn parent_terms(&mut self, index: usize, it: &Iteration, quoted: bool) -> Result<Vec<Term>> {
        let mapping: &'m Mapping = self.mapping;
        let tm = &mapping.triples_maps[index];
        let base = tm
            .base_iri
            .as_deref()
            .unwrap_or(&self.options.base_iri)
            .to_owned();
        if !quoted {
            return self.terms(&tm.subject_map.term_map, it, &base);
        }
        self.capture.push(Vec::new());
        let generated = self.triples_map(index, it, &base);
        let triples = self.capture.pop().expect("pushed above");
        generated?;
        Ok(triples
            .into_iter()
            .map(|t| Term::Triple(Box::new(t)))
            .collect())
    }

    fn index_parent(
        &mut self,
        r: &RefObjectMap,
        iterations: Iterations<'m>,
        quoted: bool,
    ) -> Result<JoinIndex> {
        let mut index: JoinIndex = HashMap::new();
        for it in iterations {
            let it = &it?;
            let mut sides = Vec::new();
            for jc in &r.join_conditions {
                sides.push(self.lexicals(&jc.parent, it)?);
            }
            let keys = cartesian(&sides);
            if keys.is_empty() {
                continue;
            }
            let subjects = self.parent_terms(r.parent, it, quoted)?;
            for k in keys {
                index.entry(k).or_default().extend(subjects.iter().cloned());
            }
        }
        Ok(index)
    }

    /// The lexical forms an expression produces, for join comparison.
    fn lexicals(&self, expr: &Expression, it: &Iteration) -> Result<Vec<String>> {
        Ok(self
            .evaluate(expr, it, TermType::Literal)?
            .into_iter()
            .map(|v| match v {
                Value_::Constant(Term::Iri(i)) => i,
                Value_::Constant(Term::Literal(l)) => l.value,
                Value_::Constant(Term::BlankNode(b)) => b,
                Value_::Constant(t @ Term::Triple(_)) => t.to_string(),
                Value_::Natural(n) => n.lexical,
                Value_::Text(t) => t,
            })
            .collect())
    }

    fn graphs(
        &mut self,
        maps: &'m [TermMap],
        it: &Iteration,
        base: &str,
    ) -> Result<Vec<GraphOut<'m>>> {
        let mut out = Vec::new();
        for m in maps {
            for t in self.terms(m, it, base)? {
                let g = if t.as_iri() == Some(DEFAULT_GRAPH) {
                    None
                } else {
                    Some(t)
                };
                out.push((g, m.targets.as_slice()));
            }
        }
        Ok(out)
    }

    /// Adds a triple to each target graph. `targets` are the logical targets
    /// of the subject, predicate and object maps; each graph adds those of
    /// the graph map that produced it.
    fn emit(
        &mut self,
        s: &Term,
        p: &Term,
        o: &Term,
        graphs: &[GraphOut<'m>],
        targets: [&[usize]; 3],
    ) -> Result<()> {
        if let Some(captured) = self.capture.last_mut() {
            let t = Triple {
                subject: s.clone(),
                predicate: p.clone(),
                object: o.clone(),
            };
            if !captured.contains(&t) {
                captured.push(t);
            }
            return Ok(());
        }
        for (g, g_t) in graphs {
            // Blank nodes are scoped to one graph: the same label in two graphs
            // must be two different blank nodes.
            let (s, o) = match g {
                Some(Term::Iri(gi)) if s.is_blank() || o.is_blank() => {
                    let n = self.graph_ids.len();
                    let id = *self.graph_ids.entry(gi.clone()).or_insert(n);
                    (scope_blank(s, id), scope_blank(o, id))
                }
                _ => (s.clone(), o.clone()),
            };
            let quad = Quad {
                subject: s,
                predicate: p.clone(),
                object: o,
                graph: g.clone(),
            };
            let fp = fingerprint(&quad);
            if targets.iter().all(|t| t.is_empty()) && g_t.is_empty() {
                self.deliver(quad, fp, Destination::Default)?;
                continue;
            }
            let mut dests: Vec<usize> = targets
                .iter()
                .flat_map(|t| t.iter())
                .chain(g_t.iter())
                .copied()
                .collect();
            dests.sort_unstable();
            dests.dedup();
            let (last, rest) = dests.split_last().expect("at least one target");
            for d in rest {
                self.deliver(quad.clone(), fp, Destination::Target(*d))?;
            }
            self.deliver(quad, fp, Destination::Target(*last))?;
        }
        Ok(())
    }

    fn deliver(&mut self, quad: Quad, fp: u128, dest: Destination) -> Result<()> {
        // One 16-byte key per (quad, destination): the destination is mixed
        // into the fingerprint. A (u128, u32) pair would pad to 32 bytes and
        // double the memory of the set.
        let key =
            fp ^ u128::from(dest.code()).wrapping_mul(0x9e37_79b9_7f4a_7c15_f39c_c060_5ced_c835);
        if self.seen.insert(key) {
            (self.sink)(quad, dest)?;
        }
        Ok(())
    }

    fn evaluate(
        &self,
        expr: &Expression,
        it: &Iteration,
        term_type: TermType,
    ) -> Result<Vec<Value_>> {
        match expr {
            Expression::Constant(t) => Ok(vec![Value_::Constant(t.clone())]),
            Expression::Reference(r) => Ok(self
                .values(it, r)?
                .into_iter()
                .map(Value_::Natural)
                .collect()),
            Expression::Template(t) => Ok(self
                .template(t, it, term_type)?
                .into_iter()
                .map(Value_::Text)
                .collect()),
            Expression::Function(call) => self.call(call, it),
        }
    }

    fn template(&self, t: &Template, it: &Iteration, term_type: TermType) -> Result<Vec<String>> {
        let mut columns: Vec<Vec<String>> = Vec::new();
        for part in &t.parts {
            columns.push(match part {
                TemplatePart::Text(s) => vec![s.clone()],
                TemplatePart::Reference(r) => {
                    let values = self.values(it, r)?;
                    if values.is_empty() {
                        return Ok(Vec::new());
                    }
                    values
                        .into_iter()
                        .map(|v| term::encode_for_template(&v.lexical, term_type))
                        .collect()
                }
            });
        }
        Ok(cartesian(&columns)
            .into_iter()
            .map(|parts| parts.concat())
            .collect())
    }

    /// The generated RDF terms of a term map for one iteration.
    fn terms(&mut self, m: &TermMap, it: &Iteration, base: &str) -> Result<Vec<Term>> {
        if let Some(c) = &m.condition
            && !self
                .call(c, it)?
                .iter()
                .any(|v| matches!(v, Value_::Natural(n) if n.lexical == "true"))
        {
            return Ok(Vec::new());
        }
        if let Some(q) = &m.quoted {
            return self.joined(q, it, true);
        }
        let Some(expr) = &m.expression else {
            self.fresh_blank += 1;
            return Ok(vec![Term::BlankNode(format!("f{}", self.fresh_blank))]);
        };
        let values = self.evaluate(expr, it, m.term_type)?;
        if values.is_empty() {
            return Ok(Vec::new());
        }
        if let Expression::Constant(_) = expr {
            return Ok(values
                .into_iter()
                .filter_map(|v| {
                    if let Value_::Constant(t) = v {
                        Some(t)
                    } else {
                        None
                    }
                })
                .collect());
        }
        let mut out = Vec::new();
        match m.term_type {
            TermType::Iri | TermType::UnsafeIri | TermType::Uri | TermType::UnsafeUri => {
                for v in values {
                    let s = text_of(&v);
                    if matches!(m.term_type, TermType::UnsafeIri | TermType::UnsafeUri) {
                        out.push(term::make_unsafe_iri(&s, base));
                        continue;
                    }
                    let t = term::make_iri(&s, base)?;
                    if m.term_type == TermType::Uri && !t.as_iri().is_some_and(str::is_ascii) {
                        return Err(Error::Data(format!("{s:?} is not a valid URI")));
                    }
                    out.push(t);
                }
            }
            TermType::BlankNode => out.extend(values.iter().map(|v| term::blank_node(&text_of(v)))),
            TermType::Literal => {
                if let Some(lm) = &m.language_map {
                    let langs = self.terms(lm, it, base)?;
                    for v in &values {
                        for l in &langs {
                            let tag = l.as_literal().map(|l| l.value.as_str()).unwrap_or_default();
                            if !term::is_valid_language_tag(tag) {
                                return Err(Error::Data(format!("invalid language tag {tag:?}")));
                            }
                            out.push(Term::Literal(Literal::lang(text_of(v), tag)));
                        }
                    }
                } else if let Some(dm) = &m.datatype_map {
                    let dts = self.terms(dm, it, base)?;
                    for v in &values {
                        for d in &dts {
                            let dt = d.as_iri().ok_or_else(|| {
                                Error::Data(format!("datatype {d} is not an IRI"))
                            })?;
                            out.push(term::typed_literal(&text_of(v), dt)?);
                        }
                    }
                } else {
                    for v in values {
                        out.push(match v {
                            Value_::Natural(n) => {
                                Term::Literal(Literal::typed(n.lexical, n.datatype))
                            }
                            other => Term::Literal(Literal::simple(text_of(&other))),
                        });
                    }
                }
            }
        }
        Ok(out)
    }
}

/// A 128-bit fingerprint of a quad: two SipHash-1-3 values with different
/// keys. Keeping fingerprints instead of quads makes duplicate removal cost
/// 16 bytes per distinct quad.
///
/// A hand-written multiply-rotate hash was tried here and dropped: on the
/// 1M-row benchmark it merged about 99,000 distinct quads.
fn fingerprint(q: &Quad) -> u128 {
    let mut a = DefaultHasher::new();
    q.hash(&mut a);
    let mut b = DefaultHasher::new();
    0x9e37_79b9_7f4a_7c15_u64.hash(&mut b);
    q.hash(&mut b);
    (u128::from(a.finish()) << 64) | u128::from(b.finish())
}

/// The rows of a CSV file read with a CSVW dialect.
fn csv_rows<'a>(
    reader: Box<dyn std::io::BufRead + 'a>,
    dialect: &CsvDialect,
) -> Result<Iterations<'a>> {
    let rows = rmlrs_source::csv::Rows::with_dialect(reader, dialect.clone())?;
    Ok(Box::new(rows.map(|r| r.map(Iteration::Csv))))
}

/// The file a `csvw:url` names. CSVW resolves a relative URL against the
/// location of the metadata, here the mapping document. HTTP(S) URLs are
/// fetched instead (see `open`); other URL schemes are not supported.
fn csvw_path(mapping_dir: &std::path::Path, url: &str) -> Result<PathBuf> {
    if let Some(path) = url.strip_prefix("file://") {
        return Ok(PathBuf::from(path));
    }
    let scheme = url.split_once(':').map(|(s, _)| s).filter(|s| {
        // One letter is a Windows drive, not a scheme.
        s.len() > 1
            && s.starts_with(|c: char| c.is_ascii_alphabetic())
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    });
    if scheme.is_some() {
        return Err(Error::Unsupported(format!(
            "csvw:url <{url}>: only local files and HTTP(S) are read"
        )));
    }
    Ok(mapping_dir.join(url))
}

/// The body of a remote document.
#[cfg(feature = "http")]
fn fetch(remote: &RemoteSource) -> Result<Box<dyn std::io::Read>> {
    rmlrs_source::http::get(remote)
}

#[cfg(not(feature = "http"))]
fn fetch(remote: &RemoteSource) -> Result<Box<dyn std::io::Read>> {
    Err(Error::Unsupported(format!(
        "remote source {} (needs the `http` feature, or the document in Options::in_memory)",
        remote.url
    )))
}

/// The results of a SELECT query sent to a SPARQL endpoint.
#[cfg(feature = "http")]
fn sparql_query(
    endpoint: &str,
    query: &str,
    format: SparqlResultsFormat,
) -> Result<Box<dyn std::io::Read>> {
    rmlrs_source::http::sparql_query(endpoint, query, format)
}

#[cfg(not(feature = "http"))]
fn sparql_query(endpoint: &str, _: &str, _: SparqlResultsFormat) -> Result<Box<dyn std::io::Read>> {
    Err(Error::Unsupported(format!(
        "SPARQL endpoint {endpoint} (needs the `http` feature)"
    )))
}

fn with_path(e: Error, path: &str) -> Error {
    match e {
        Error::Source(m) if !m.starts_with(path) => Error::Source(format!("{path}: {m}")),
        other => other,
    }
}

fn text_of(v: &Value_) -> String {
    match v {
        Value_::Constant(Term::Literal(l)) => l.value.clone(),
        Value_::Constant(t) => t.to_string(),
        Value_::Natural(n) => n.lexical.clone(),
        Value_::Text(t) => t.clone(),
    }
}

fn scope_blank(t: &Term, graph: usize) -> Term {
    match t {
        Term::BlankNode(b) => Term::BlankNode(format!("{b}g{graph}")),
        other => other.clone(),
    }
}

/// Target graphs: the default graph when no graph maps exist at all,
/// otherwise every generated graph term, with the targets of its graph map.
/// The same graph may appear twice with different targets; duplicate quads
/// are removed per destination.
fn target_graphs<'m>(
    sgm: &[TermMap],
    pogm: &[TermMap],
    subject_graphs: &[GraphOut<'m>],
    pom_graphs: &[GraphOut<'m>],
) -> Vec<GraphOut<'m>> {
    if sgm.is_empty() && pogm.is_empty() {
        return vec![(None, &[])];
    }
    subject_graphs.iter().chain(pom_graphs).cloned().collect()
}

/// The address of a referencing object map, which keys its join index.
/// The mapping is borrowed for the whole run, so addresses do not change.
fn join_key(r: &RefObjectMap) -> usize {
    std::ptr::from_ref(r) as usize
}

/// The object maps of a triples map, including the members of its gather
/// maps (RML-CC) at any depth, in the subject map as well.
fn object_maps_of(tm: &TriplesMap) -> Vec<&ObjectMap> {
    let mut out: Vec<&ObjectMap> = tm
        .predicate_object_maps
        .iter()
        .flat_map(|pom| &pom.object_maps)
        .collect();
    if let Some(g) = &tm.subject_map.term_map.gather {
        out.extend(&g.items);
    }
    let mut i = 0;
    while i < out.len() {
        if let ObjectMap::Term(m) = out[i]
            && let Some(g) = &m.gather
        {
            out.extend(&g.items);
        }
        i += 1;
    }
    out
}

/// The star maps (RML-star) of a triples map: in its subject map and its
/// object maps.
fn star_maps_of(tm: &TriplesMap) -> Vec<&RefObjectMap> {
    let mut out: Vec<&RefObjectMap> = tm
        .subject_map
        .term_map
        .quoted
        .as_deref()
        .into_iter()
        .collect();
    for om in object_maps_of(tm) {
        if let ObjectMap::Term(m) = om
            && let Some(q) = &m.quoted
        {
            out.push(q);
        }
    }
    out
}

/// Rejects a triples map that quotes itself, directly or through others:
/// its quoted triples would never end.
fn check_quoting_cycles(mapping: &Mapping) -> Result<()> {
    fn visit(mapping: &Mapping, i: usize, path: &mut Vec<usize>) -> Result<()> {
        if path.contains(&i) {
            return Err(Error::Mapping(format!(
                "{} quotes its own triples",
                mapping.triples_maps[i].node
            )));
        }
        path.push(i);
        for q in star_maps_of(&mapping.triples_maps[i]) {
            visit(mapping, q.parent, path)?;
        }
        path.pop();
        Ok(())
    }
    for i in 0..mapping.triples_maps.len() {
        visit(mapping, i, &mut Vec::new())?;
    }
    Ok(())
}

/// The n-ary Cartesian product; empty if any column is empty.
fn cartesian<T: Clone>(columns: &[Vec<T>]) -> Vec<Vec<T>> {
    let mut acc: Vec<Vec<T>> = vec![Vec::new()];
    for col in columns {
        let mut next = Vec::with_capacity(acc.len() * col.len());
        for prefix in &acc {
            for v in col {
                let mut p = prefix.clone();
                p.push(v.clone());
                next.push(p);
            }
        }
        acc = next;
    }
    acc
}

#[cfg(feature = "fnml")]
fn call_key(call: &FunctionCall) -> usize {
    std::ptr::from_ref(call).addr()
}

fn expr_calls<'m>(e: &'m Expression, out: &mut Vec<&'m FunctionCall>) {
    if let Expression::Function(call) = e {
        out.push(call);
        for (_, v) in &call.inputs {
            expr_calls(v, out);
        }
    }
}

fn term_map_calls<'m>(m: &'m TermMap, out: &mut Vec<&'m FunctionCall>) {
    if let Some(e) = &m.expression {
        expr_calls(e, out);
    }
    if let Some(c) = &m.condition {
        out.push(c);
        for (_, v) in &c.inputs {
            expr_calls(v, out);
        }
    }
    for d in m.datatype_map.iter().chain(&m.language_map) {
        term_map_calls(d, out);
    }
}

fn expr_refs(e: &Expression, out: &mut Vec<String>) {
    match e {
        Expression::Reference(r) => out.push(r.clone()),
        Expression::Template(t) => {
            for p in &t.parts {
                if let TemplatePart::Reference(r) = p {
                    out.push(r.clone());
                }
            }
        }
        Expression::Function(call) => {
            for (_, v) in &call.inputs {
                expr_refs(v, out);
            }
        }
        Expression::Constant(_) => {}
    }
}

fn collect_refs(m: &TermMap, out: &mut Vec<String>) {
    if let Some(e) = &m.expression {
        expr_refs(e, out);
    }
    if let Some(c) = &m.condition {
        for (_, v) in &c.inputs {
            expr_refs(v, out);
        }
    }
    if let Some(d) = &m.datatype_map {
        collect_refs(d, out);
    }
    if let Some(l) = &m.language_map {
        collect_refs(l, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    const PREFIX: &str = r#"
        @prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .
        ex:TM a rml:TriplesMap ;
          rml:logicalSource [ rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "data.json" ] ;
                              rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" ] ;
    "#;

    fn run_inline(body: &str, data: &str) -> Result<Vec<Quad>> {
        let mapping = format!("{PREFIX}{body}");
        let tag = mapping.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        let dir = std::env::temp_dir().join(format!("rmlrs-engine-{}-{tag:x}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("data.json"), data).unwrap();
        let m = rmlrs_parser::parse_mapping(&mapping, None)?;
        let out = run(
            &m,
            &Options {
                base_iri: "http://example.com/".into(),
                mapping_dir: dir.clone(),
                working_dir: dir.clone(),
                ..Default::default()
            },
        );
        std::fs::remove_dir_all(&dir).ok();
        out
    }

    /// The literal values of the RDF list starting at `head`, following
    /// `rdf:rest`; `None` if the list is ill-formed or not closed.
    fn list_values(out: &[Quad], head: &Term) -> Option<Vec<String>> {
        let one = |s: &Term, p: &str| {
            let mut v = out
                .iter()
                .filter(|q| &q.subject == s && q.predicate.as_iri() == Some(p));
            match (v.next(), v.next()) {
                (Some(q), None) => Some(q.object.clone()),
                _ => None,
            }
        };
        let mut values = Vec::new();
        let mut cell = head.clone();
        while cell.as_iri() != Some(RDF_NIL) {
            values.push(one(&cell, RDF_FIRST)?.as_literal()?.value.clone());
            cell = one(&cell, RDF_REST)?;
        }
        Some(values)
    }

    /// Runs a whole mapping document over files written to a temporary directory.
    fn run_files(mapping: &str, files: &[(&str, &str)]) -> Result<Vec<Quad>> {
        let files: Vec<(&str, &[u8])> = files.iter().map(|(n, t)| (*n, t.as_bytes())).collect();
        run_byte_files(mapping, &files)
    }

    /// Same as [`run_files`], for files that are not UTF-8 text.
    fn run_byte_files(mapping: &str, files: &[(&str, &[u8])]) -> Result<Vec<Quad>> {
        let tag = mapping.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x100_0000_01b3)
        });
        let dir = std::env::temp_dir().join(format!("rmlrs-engine-{}-{tag:x}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in files {
            std::fs::write(dir.join(name), text).unwrap();
        }
        let m = rmlrs_parser::parse_mapping(mapping, None)?;
        let out = run(
            &m,
            &Options {
                base_iri: "http://example.com/".into(),
                mapping_dir: dir.clone(),
                working_dir: dir.clone(),
                ..Default::default()
            },
        );
        std::fs::remove_dir_all(&dir).ok();
        out
    }

    const VIEWS: &str = r##"
        @prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .
        ex:people a rml:LogicalSource ; rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" ;
          rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "people.json" ] .
        ex:years a rml:LogicalSource ; rml:referenceFormulation rml:CSV ;
          rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "years.csv" ] .
        ex:items a rml:LogicalView ; rml:viewOn ex:people ;
          rml:field [ a rml:ExpressionField ; rml:fieldName "name" ; rml:reference "$.name" ] ;
          rml:field [ a rml:IterableField ; rml:fieldName "item" ; rml:iterator "$.items[*]" ;
                      rml:field [ a rml:ExpressionField ; rml:fieldName "type" ; rml:reference "$.type" ] ] .
        ex:born a rml:LogicalView ; rml:viewOn ex:years ;
          rml:field [ a rml:ExpressionField ; rml:fieldName "who" ; rml:reference "who" ] ;
          JOIN [ rml:parentLogicalView ex:items ;
                 rml:joinCondition [ rml:child "who" ; rml:parent "name" ] ;
                 rml:field [ a rml:ExpressionField ; rml:fieldName "thing" ; rml:reference "item.type" ] ] .
        ex:TM a rml:TriplesMap ; rml:logicalSource ex:born ;
          rml:subjectMap [ rml:template "p/{who}" ] ;
          rml:predicateObjectMap [ rml:predicate ex:has ; rml:objectMap [ rml:template "t/{thing.#}/{thing}" ] ] ;
          rml:predicateObjectMap [ rml:predicate ex:row ; rml:objectMap [ rml:reference "#" ] ] .
    "##;
    const PEOPLE: &str =
        r#"[{"name": "a", "items": [{"type": "x"}, {"type": "y"}]}, {"name": "b", "items": []}]"#;
    const YEARS: &str = "who\na\nb\nc\n";

    fn view_triples(join: &str) -> BTreeSet<String> {
        run_files(
            &VIEWS.replace("JOIN", join),
            &[("people.json", PEOPLE), ("years.csv", YEARS)],
        )
        .unwrap()
        .iter()
        .map(|q| format!("{} {} {}", q.subject, q.predicate, q.object))
        .collect()
    }

    #[test]
    fn logical_view_fields_indexes_and_joins() {
        let iri = |s: &str| format!("<http://example.com/{s}>");
        let int = |n: u8| format!("\"{n}\"^^<http://www.w3.org/2001/XMLSchema#integer>");
        let has = |p: &str, t: &str| format!("{} {} {}", iri(p), iri("has"), iri(t));
        let row = |p: &str, n: u8| format!("{} {} {}", iri(p), iri("row"), int(n));
        // "a" has two items (indexes 0 and 1); "b" has none, so its "thing"
        // is empty; "c" is not in the parent view at all.
        let left = view_triples("rml:leftJoin");
        let want: BTreeSet<String> = [
            has("p/a", "t/0/x"),
            has("p/a", "t/1/y"),
            row("p/a", 0),
            row("p/b", 1),
            row("p/c", 2),
        ]
        .into_iter()
        .collect();
        assert_eq!(left, want);
        // An inner join keeps "b": it matches a parent iteration, one whose
        // iterable field has no records.
        let inner = view_triples("rml:innerJoin");
        assert_eq!(
            inner,
            want.into_iter().filter(|t| !t.contains("/p/c")).collect()
        );
    }

    #[test]
    fn logical_view_references_must_name_keys() {
        let bad = VIEWS
            .replace("JOIN", "rml:leftJoin")
            .replace(r##"rml:reference "#" ]"##, r#"rml:reference "item" ]"#);
        let err = rmlrs_parser::parse_mapping(&bad, None)
            .and_then(|m| run(&m, &Options::default()))
            .unwrap_err();
        assert!(err.to_string().contains("no referenceable key"), "{err}");
    }

    const STAR: &str = r#"
        @prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .
        ex:rows a rml:LogicalSource ; rml:referenceFormulation rml:CSV ;
          rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "rows.csv" ] .
        ex:scores a rml:LogicalSource ; rml:referenceFormulation rml:CSV ;
          rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "scores.csv" ] .
        ex:Type a KIND ; rml:logicalSource ex:rows ;
          rml:subjectMap [ rml:template "{who}" ] ;
          rml:predicateObjectMap [ rml:predicate ex:is ; rml:objectMap [ rml:template "{what}" ] ] .
        ex:Said a rml:NonAssertedTriplesMap ; rml:logicalSource ex:rows ;
          rml:subjectMap [ rml:template "{by}" ] ;
          rml:predicateObjectMap [ rml:predicate ex:says ; rml:objectMap [ rml:quotedTriplesMap ex:Type ] ] .
        ex:Score rml:logicalSource ex:scores ;
          rml:subjectMap [ rml:quotedTriplesMap ex:Said ;
                           rml:joinCondition [ rml:child "id" ; rml:parent "id" ] ] ;
          rml:predicateObjectMap [ rml:predicate ex:score ; rml:objectMap [ rml:reference "score" ] ] .
    "#;

    fn star_lines(kind: &str) -> BTreeSet<String> {
        run_files(
            &STAR.replace("KIND", kind),
            &[
                ("rows.csv", "id,who,what,by\n1,a,b,x\n2,c,d,y\n"),
                ("scores.csv", "id,score\n2,5\n"),
            ],
        )
        .unwrap()
        .iter()
        .map(|q| format!("{} {} {}", q.subject, q.predicate, q.object))
        .collect()
    }

    #[test]
    fn quoted_triples_asserted_or_not_and_joined() {
        let e = |s: &str| format!("<http://example.com/{s}>");
        let said = format!(
            "<< {} {} << {} {} {} >> >>",
            e("y"),
            e("says"),
            e("c"),
            e("is"),
            e("d")
        );
        let score = format!("{said} {} \"5\"", e("score"));
        // Only the iteration joined with a score is quoted; ex:Said is never asserted.
        assert_eq!(
            star_lines("rml:NonAssertedTriplesMap"),
            BTreeSet::from([score.clone()])
        );
        let asserted = star_lines("rml:AssertedTriplesMap");
        assert_eq!(
            asserted,
            BTreeSet::from([
                score,
                format!("{} {} {}", e("a"), e("is"), e("b")),
                format!("{} {} {}", e("c"), e("is"), e("d")),
            ])
        );
    }

    #[test]
    fn a_triples_map_cannot_quote_itself() {
        let looped = STAR.replace("KIND", "rml:TriplesMap").replace(
            "rml:objectMap [ rml:template \"{what}\" ]",
            "rml:objectMap [ rml:quotedTriplesMap ex:Said ]",
        );
        let err = rmlrs_parser::parse_mapping(&looped, None)
            .and_then(|m| run(&m, &Options::default()))
            .unwrap_err();
        assert!(err.to_string().contains("quotes its own triples"), "{err}");
    }

    #[test]
    fn legacy_rml_reads_relative_json_paths_as_plain_values() {
        let old = r#"
            @prefix rr: <http://www.w3.org/ns/r2rml#> . @prefix rml: <http://semweb.mmlab.be/ns/rml#> .
            @prefix ql: <http://semweb.mmlab.be/ns/ql#> . @prefix ex: <http://example.com/> .
            ex:TM a rr:TriplesMap ;
              rml:logicalSource [ rml:source "people.json" ; rml:referenceFormulation ql:JSONPath ; rml:iterator "$[*]" ] ;
              rr:subjectMap [ rr:template "p/{id}" ] ;
              rr:predicateObjectMap [ rr:predicate ex:age ; rr:objectMap [ rml:reference "age" ] ] ."#;
        let out = run_files(old, &[("people.json", r#"[{"id": 1, "age": 30.50}]"#)]).unwrap();
        let lines: Vec<String> = out.iter().map(|q| q.to_string()).collect();
        // The JSON text of the number, without a datatype: what old RML gave.
        assert_eq!(
            lines,
            ["<http://example.com/p/1> <http://example.com/age> \"30.50\" ."]
        );
    }

    #[test]
    fn named_collections_are_appended_across_iterations() {
        let out = run_inline(
            r#"rml:subjectMap [ rml:template "e/{$.id}" ] ;
               rml:predicateObjectMap [ rml:predicate ex:list ;
                 rml:objectMap [ rml:template "l/{$.id}" ; rml:gather ( [ rml:reference "$.v.*" ] ) ;
                                 rml:gatherAs <http://www.w3.org/1999/02/22-rdf-syntax-ns#List> ] ] ;
               rml:predicateObjectMap [ rml:predicate ex:seq ;
                 rml:objectMap [ rml:template "s/{$.id}" ; rml:gather ( [ rml:reference "$.v.*" ] ) ;
                                 rml:gatherAs <http://www.w3.org/1999/02/22-rdf-syntax-ns#Seq> ] ] ."#,
            r#"[{"id": "a", "v": ["1", "2"]}, {"id": "b", "v": ["3"]}, {"id": "a", "v": ["4"]}]"#,
        )
        .unwrap();
        let iri = |s: &str| Term::Iri(format!("http://example.com/{s}"));
        assert_eq!(list_values(&out, &iri("l/a")).unwrap(), ["1", "2", "4"]);
        assert_eq!(list_values(&out, &iri("l/b")).unwrap(), ["3"]);
        let seq: BTreeSet<(String, String)> = out
            .iter()
            .filter(|q| q.subject == iri("s/a") && q.predicate.as_iri() != Some(RDF_TYPE))
            .map(|q| {
                (
                    q.predicate.to_string(),
                    q.object.as_literal().unwrap().value.clone(),
                )
            })
            .collect();
        let rdf = |n: u8| format!("<{RDF}_{n}>");
        let want = [(rdf(1), "1"), (rdf(2), "2"), (rdf(3), "4")]
            .map(|(p, o)| (p, o.to_owned()))
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(seq, want);
    }

    #[test]
    fn gather_strategies_and_empty_lists() {
        let gather = |extra: &str| {
            format!(
                r#"rml:subjectMap [ rml:template "e/{{$.id}}" ] ;
                   rml:predicateObjectMap [ rml:predicate ex:p ;
                     rml:objectMap [ rml:gather ( [ rml:reference "$.a.*" ] [ rml:reference "$.b.*" ] ) ;
                                     rml:gatherAs <http://www.w3.org/1999/02/22-rdf-syntax-ns#List> {extra} ] ] ."#
            )
        };
        let lists = |out: &[Quad]| -> BTreeSet<Vec<String>> {
            out.iter()
                .filter(|q| q.predicate.as_iri() == Some("http://example.com/p"))
                .map(|q| list_values(out, &q.object).unwrap())
                .collect()
        };
        let data = r#"[{"id": "x", "a": ["1", "2"], "b": ["3"]}, {"id": "y", "a": [], "b": []}]"#;
        let appended = run_inline(&gather(""), data).unwrap();
        assert_eq!(
            lists(&appended),
            BTreeSet::from([vec!["1".into(), "2".into(), "3".into()]])
        );
        let product = run_inline(&gather("; rml:strategy rml:cartesianProduct"), data).unwrap();
        assert_eq!(
            lists(&product),
            BTreeSet::from([
                vec!["1".to_owned(), "3".to_owned()],
                vec!["2".to_owned(), "3".to_owned()]
            ])
        );
        // Nothing gathered for "y": no triple, unless empty lists are allowed.
        let allowed = run_inline(&gather("; rml:allowEmptyListAndContainer true"), data).unwrap();
        let y = Term::Iri("http://example.com/e/y".into());
        assert!(!appended.iter().any(|q| q.subject == y));
        assert!(
            allowed
                .iter()
                .any(|q| q.subject == y && q.object.as_iri() == Some(RDF_NIL))
        );
    }

    #[test]
    fn blank_nodes_are_scoped_per_graph() {
        let out = run_inline(
            r#"rml:subjectMap [ rml:template "{$.id}" ; rml:termType rml:BlankNode ;
                                rml:graphMap [ rml:template "http://example.com/g/{$.g}" ] ] ;
               rml:predicateObjectMap [ rml:predicate ex:p ; rml:object "x" ] ."#,
            r#"[{"id": "a", "g": 1}, {"id": "a", "g": 2}]"#,
        )
        .unwrap();
        let subjects: BTreeSet<_> = out.iter().map(|q| q.subject.clone()).collect();
        assert_eq!(out.len(), 2);
        assert_eq!(
            subjects.len(),
            2,
            "same label in two graphs must be two blank nodes"
        );
    }

    #[test]
    fn fresh_blank_node_per_iteration_shared_within_it() {
        let out = run_inline(
            r#"rml:subjectMap [ rml:termType rml:BlankNode ] ;
               rml:predicateObjectMap [ rml:predicate ex:p ; rml:objectMap [ rml:reference "$.a" ] ] ;
               rml:predicateObjectMap [ rml:predicate ex:q ; rml:objectMap [ rml:reference "$.b" ] ] ."#,
            r#"[{"a": 1, "b": 2}, {"a": 3, "b": 4}]"#,
        )
        .unwrap();
        let subjects: BTreeSet<_> = out.iter().map(|q| q.subject.clone()).collect();
        assert_eq!(out.len(), 4);
        assert_eq!(subjects.len(), 2);
    }

    #[test]
    fn multi_valued_template_is_a_cartesian_product() {
        let out = run_inline(
            r#"rml:subjectMap [ rml:template "http://example.com/{$.x[*]}/{$.y[*]}" ] ;
               rml:predicateObjectMap [ rml:predicate ex:p ; rml:object "o" ] ."#,
            r#"[{"x": [1, 2], "y": ["a", "b"]}]"#,
        )
        .unwrap();
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn in_memory_documents_replace_files() {
        let m = rmlrs_parser::parse_mapping(
            &format!(r#"{PREFIX} rml:subjectMap [ rml:template "http://e/{{$.a}}" ] ; rml:predicateObjectMap [ rml:predicate ex:p ; rml:object "o" ] ."#),
            None,
        )
        .unwrap();
        let mut options = Options {
            base_iri: "http://e/".into(),
            ..Default::default()
        };
        options
            .in_memory
            .insert("data.json".into(), r#"[{"a": 1}, {"a": 2}]"#.into());
        assert_eq!(run(&m, &options).unwrap().len(), 2);
    }

    #[test]
    fn fingerprints_keep_similar_quads_apart() {
        // The shape that exposed a weak hash: IRIs that differ in one digit.
        let mut seen = HashSet::new();
        let n = 300_000;
        for i in 0..n {
            let q = Quad {
                subject: Term::Iri(format!("http://example.com/person/{i}")),
                predicate: Term::iri(RDF_TYPE),
                object: Term::iri("http://example.com/ns#Person"),
                graph: None,
            };
            seen.insert(fingerprint(&q));
        }
        assert_eq!(seen.len(), n);
    }

    /// Triples maps over `a.json` and `b.json`, held in memory.
    fn run_two_sources(maps: &str, a: &str, b: &str) -> Vec<Quad> {
        let mapping = format!(
            r#"@prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .
               {maps}"#
        );
        let m = rmlrs_parser::parse_mapping(&mapping, None).unwrap();
        let mut options = Options {
            base_iri: "http://example.com/".into(),
            ..Default::default()
        };
        options.in_memory.insert("a.json".into(), a.into());
        options.in_memory.insert("b.json".into(), b.into());
        run(&m, &options).unwrap()
    }

    /// A triples map over `file`; `extra` adds to the source description.
    fn json_map(name: &str, file: &str, extra: &str) -> String {
        format!(
            r#"ex:{name} rml:logicalSource [
                 rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "{file}" {extra} ] ;
                 rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" ] ;
               rml:subjectMap [ rml:template "http://example.com/{name}/{{$.id}}" ] ;
               rml:predicateObjectMap [ rml:predicate ex:v ; rml:objectMap [ rml:reference "$.v" ] ] ."#
        )
    }

    #[test]
    fn maps_over_one_logical_source_are_grouped_in_mapping_order() {
        let maps = [
            json_map("A", "a.json", ""),
            json_map("B", "b.json", ""),
            json_map("C", "a.json", ""),
            // Same file, different rml:null: a different logical source.
            json_map("D", "a.json", r#"; rml:null "x""#),
        ]
        .concat();
        let m = rmlrs_parser::parse_mapping(
            &format!(
                r#"@prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> . {maps}"#
            ),
            None,
        )
        .unwrap();
        assert_eq!(
            source_groups(&m.triples_maps),
            vec![vec![0, 2], vec![1], vec![3]]
        );
    }

    #[test]
    fn grouped_maps_take_turns_on_each_iteration() {
        let maps = [json_map("A", "a.json", ""), json_map("B", "a.json", "")].concat();
        let out = run_two_sources(&maps, r#"[{"id": 1, "v": "p"}, {"id": 2, "v": "q"}]"#, "[]");
        let subjects: Vec<String> = out.iter().map(|q| q.subject.to_string()).collect();
        assert_eq!(
            subjects,
            [
                "<http://example.com/A/1>",
                "<http://example.com/B/1>",
                "<http://example.com/A/2>",
                "<http://example.com/B/2>",
            ]
        );
    }

    #[test]
    fn each_group_keeps_its_own_null_values() {
        let maps = [
            json_map("A", "a.json", ""),
            json_map("B", "a.json", r#"; rml:null "x""#),
            json_map("C", "b.json", ""),
        ]
        .concat();
        let out = run_two_sources(
            &maps,
            r#"[{"id": 1, "v": "x"}]"#,
            r#"[{"id": 2, "v": "x"}]"#,
        );
        let subjects: BTreeSet<String> = out.iter().map(|q| q.subject.to_string()).collect();
        assert_eq!(
            subjects,
            BTreeSet::from([
                "<http://example.com/A/1>".to_owned(),
                "<http://example.com/C/2>".to_owned(),
            ])
        );
    }

    #[test]
    fn missing_source_is_a_source_error() {
        let m = rmlrs_parser::parse_mapping(
            &format!(r#"{PREFIX} rml:subjectMap [ rml:template "http://e/{{$.a}}" ] ."#),
            None,
        )
        .unwrap();
        let dir = std::env::temp_dir().join("rmlrs-engine-missing");
        let r = run(
            &m,
            &Options {
                base_iri: "http://e/".into(),
                mapping_dir: dir.clone(),
                working_dir: dir,
                ..Default::default()
            },
        );
        assert!(matches!(r, Err(Error::Source(_))));
    }

    /// A triples map over the source `source`, with the CSV columns ID and
    /// Name (or the JSON keys of the same names).
    fn people(source: &str, formulation: &str) -> String {
        let (id, name, iterator) = if formulation == "JSONPath" {
            ("$.ID", "$.Name", r#"; rml:iterator "$[*]""#)
        } else {
            ("ID", "Name", "")
        };
        format!(
            r#"@prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .
               @prefix csvw: <http://www.w3.org/ns/csvw#> .
               ex:TM rml:logicalSource [ rml:source {source} ;
                   rml:referenceFormulation rml:{formulation} {iterator} ] ;
                 rml:subjectMap [ rml:template "http://example.com/{{{id}}}" ] ;
                 rml:predicateObjectMap [ rml:predicate ex:name ; rml:objectMap [ rml:reference "{name}" ] ] ."#
        )
    }

    /// The (subject, name) pairs of the output.
    fn names(out: &[Quad]) -> BTreeSet<(String, String)> {
        out.iter()
            .map(|q| {
                let o = q.object.as_literal().expect("a literal").value.clone();
                (q.subject.to_string(), o)
            })
            .collect()
    }

    fn pairs(v: &[(&str, &str)]) -> BTreeSet<(String, String)> {
        v.iter()
            .map(|(s, o)| (format!("<http://example.com/{s}>"), (*o).to_owned()))
            .collect()
    }

    fn utf16(text: &str, little_endian: bool) -> Vec<u8> {
        std::iter::once(0xFEFF)
            .chain(text.encode_utf16())
            .flat_map(|u| {
                if little_endian {
                    u.to_le_bytes()
                } else {
                    u.to_be_bytes()
                }
            })
            .collect()
    }

    #[test]
    fn csvw_table_with_dialect_and_null() {
        let m = people(
            r#"[ a csvw:Table ; csvw:url "s.csv" ; csvw:null "NULL" ;
                 csvw:dialect [ a csvw:Dialect ; csvw:delimiter ";" ; csvw:trim true ] ]"#,
            "CSV",
        );
        let out = run_files(&m, &[("s.csv", "ID;Name\n10; Venus\n11;NULL\n12;null\n")]).unwrap();
        assert_eq!(names(&out), pairs(&[("10", "Venus"), ("12", "null")]));
    }

    #[test]
    fn rml_null_takes_precedence_over_csvw_null() {
        let m = people(
            r#"[ a rml:Source, csvw:Table ; csvw:url "s.csv" ; csvw:null "NULL" ; rml:null "x" ]"#,
            "CSV",
        );
        let out = run_files(&m, &[("s.csv", "ID,Name\n10,NULL\n11,x\n")]).unwrap();
        assert_eq!(names(&out), pairs(&[("10", "NULL")]));
    }

    #[test]
    fn utf16_sources() {
        let json = r#"[{"ID": 1, "Name": "Vénus 𝄞"}]"#;
        let m = people(
            r#"[ a rml:RelativePathSource ; rml:path "s.json" ; rml:encoding rml:UTF-16 ]"#,
            "JSONPath",
        );
        for le in [true, false] {
            let out = run_byte_files(&m, &[("s.json", &utf16(json, le))]).unwrap();
            assert_eq!(names(&out), pairs(&[("1", "Vénus 𝄞")]));
        }
        // csvw:encoding, and rml:encoding taking precedence over it.
        let csv = utf16("ID,Name\n10,Vénus\n", false);
        for source in [
            r#"[ a csvw:Table ; csvw:url "s.csv" ; csvw:dialect [ csvw:encoding "UTF-16" ] ]"#,
            r#"[ a csvw:Table ; csvw:url "s.csv" ; rml:encoding rml:UTF-16 ;
                 csvw:dialect [ csvw:encoding "utf-8" ] ]"#,
        ] {
            let out = run_byte_files(&people(source, "CSV"), &[("s.csv", &csv)]).unwrap();
            assert_eq!(names(&out), pairs(&[("10", "Vénus")]));
        }
        // Read as UTF-8, the same bytes are not text.
        let m = people(r#"[ a csvw:Table ; csvw:url "s.csv" ]"#, "CSV");
        assert!(matches!(
            run_byte_files(&m, &[("s.csv", &csv)]),
            Err(Error::Source(_))
        ));
    }

    #[test]
    fn remote_csvw_urls_can_be_given_in_memory() {
        let url = "http://example.org/s.csv";
        let table =
            format!(r#"[ a csvw:Table ; csvw:url "{url}" ; csvw:dialect [ csvw:delimiter ";" ] ]"#);
        let m = people(&table, "CSV");
        let m = rmlrs_parser::parse_mapping(&m, None).unwrap();
        let mut options = Options {
            base_iri: "http://example.com/".into(),
            ..Default::default()
        };
        // Without the client, a remote table must come from memory.
        #[cfg(not(feature = "http"))]
        assert!(matches!(run(&m, &options), Err(Error::Unsupported(_))));
        options
            .in_memory
            .insert(url.into(), "ID;Name\n10;Venus, W\n".into());
        assert_eq!(
            names(&run(&m, &options).unwrap()),
            pairs(&[("10", "Venus, W")])
        );
    }

    #[test]
    fn csvw_tables_are_read_as_csv() {
        let m = people(r#"[ a csvw:Table ; csvw:url "s.json" ]"#, "JSONPath");
        let r = run_files(&m, &[("s.json", "[]")]);
        assert!(matches!(r, Err(Error::Mapping(_))));
    }

    #[test]
    fn csvw_url_paths() {
        let dir = std::path::Path::new("/maps");
        assert_eq!(csvw_path(dir, "a/s.csv").unwrap(), dir.join("a/s.csv"));
        assert_eq!(
            csvw_path(dir, "file:///data/s.csv").unwrap(),
            PathBuf::from("/data/s.csv")
        );
        assert!(csvw_path(dir, "https://example.org/s.csv").is_err());
    }

    /// A triples map over a remote source described by `source`.
    fn remote_mapping(source: &str) -> Mapping {
        rmlrs_parser::parse_mapping(
            &format!(
                r#"@prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .
                   @prefix dcat: <http://www.w3.org/ns/dcat#> .
                   @prefix td: <https://www.w3.org/2019/wot/td#> .
                   @prefix hctl: <https://www.w3.org/2019/wot/hypermedia#> .
                   ex:TM rml:logicalSource [ rml:source [ a rml:Source ; {source} ] ;
                       rml:referenceFormulation rml:JSONPath ; rml:iterator "$[*]" ] ;
                     rml:subjectMap [ rml:template "http://e/{{$.a}}" ] ;
                     rml:predicateObjectMap [ rml:predicate ex:p ; rml:object "o" ] ."#
            ),
            None,
        )
        .unwrap()
    }

    #[test]
    fn remote_documents_can_be_given_in_memory() {
        let url = "http://w3id.org/rml/resources/rml-io/X/data.json";
        let m = remote_mapping(&format!("dcat:downloadURL <{url}>"));
        let mut options = Options::default();
        options
            .in_memory
            .insert(url.into(), r#"[{"a": 1}, {"a": 2}]"#.into());
        assert_eq!(run(&m, &options).unwrap().len(), 2);
    }

    #[cfg(not(feature = "http"))]
    #[test]
    fn remote_documents_need_the_http_feature() {
        let m = remote_mapping("dcat:downloadURL <http://example.com/data.json>");
        assert!(matches!(
            run(&m, &Options::default()),
            Err(Error::Unsupported(_))
        ));
    }

    /// Answers HTTP requests with `body`, for the rest of the test run, and
    /// returns the URL to ask.
    #[cfg(feature = "http")]
    fn serve_once(status: &str, body: &str) -> String {
        use std::io::{BufRead, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/data.json", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(&stream);
                // Read the request to its end (headers, then a body, if any)
                // so that closing the connection does not reset it.
                let (mut line, mut length) = (String::new(), 0);
                while reader.read_line(&mut line).unwrap() > 2 {
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    line.clear();
                }
                std::io::Read::read_exact(&mut reader, &mut vec![0; length]).unwrap();
                (&stream).write_all(response.as_bytes()).unwrap();
            }
        });
        url
    }

    /// The old RML test case RMLTC0009a-SPARQL (two endpoints, a join on
    /// whole bindings), with its endpoints answering the solutions of the
    /// case's data: the translated mapping gives the case's expected output.
    #[cfg(feature = "http")]
    #[test]
    fn old_rml_sparql_sources_are_translated() {
        let students = r#"{ "head": { "vars": ["Name", "ID", "Sport"] }, "results": { "bindings": [
            { "Name": { "type": "literal", "value": "Venus Williams" },
              "ID": { "type": "literal", "value": "10" },
              "Sport": { "type": "literal", "value": "100" } },
            { "Name": { "type": "literal", "value": "Demi Moore" },
              "ID": { "type": "literal", "value": "20" } } ] } }"#;
        let sports = r#"{ "head": { "vars": ["ID", "Name"] }, "results": { "bindings": [
            { "ID": { "type": "literal", "value": "100" },
              "Name": { "type": "literal", "value": "Tennis" } } ] } }"#;
        let (ds1, ds2) = (serve_once("200 OK", students), serve_once("200 OK", sports));
        let source = |name: &str, endpoint: &str, select: &str| {
            format!(
                r#"<#{name}> a sd:Service ; sd:endpoint <{endpoint}> ;
                     sd:supportedLanguage sd:SPARQL11Query ;
                     sd:resultFormat <http://www.w3.org/ns/formats/SPARQL_Results_JSON> .
                   <#{name}LS> rml:source <#{name}> ; rml:referenceFormulation ql:JSONPath ;
                     rml:iterator "$.results.bindings[*]" ; rml:query "{select}" ."#
            )
        };
        let mapping = format!(
            r#"@prefix rr: <http://www.w3.org/ns/r2rml#> . @prefix foaf: <http://xmlns.com/foaf/0.1/> .
               @prefix rml: <http://semweb.mmlab.be/ns/rml#> . @prefix ql: <http://semweb.mmlab.be/ns/ql#> .
               @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
               @prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
               @base <http://example.com/base/> .
               {}
               {}
               <TriplesMap1> a rr:TriplesMap ; rml:logicalSource <#InputSPARQL1LS> ;
                 rr:subjectMap [ rr:template "http://example.com/resource/student_{{ID.value}}" ] ;
                 rr:predicateObjectMap [ rr:predicate foaf:name ; rr:objectMap [ rml:reference "Name.value" ] ] ;
                 rr:predicateObjectMap [ rr:predicate <http://example.com/ontology/practises> ;
                   rr:objectMap [ a rr:RefObjectMap ; rr:parentTriplesMap <TriplesMap2> ;
                     rr:joinCondition [ rr:child "Sport" ; rr:parent "ID" ] ] ] .
               <TriplesMap2> a rr:TriplesMap ; rml:logicalSource <#InputSPARQL2LS> ;
                 rr:subjectMap [ rr:template "http://example.com/resource/sport_{{ID.value}}" ] ;
                 rr:predicateObjectMap [ rr:predicate rdfs:label ; rr:objectMap [ rml:reference "Name.value" ] ] ."#,
            source(
                "InputSPARQL1",
                &ds1,
                "SELECT ?Name ?ID ?Sport WHERE { ... }"
            ),
            source("InputSPARQL2", &ds2, "SELECT ?ID ?Name WHERE { ... }"),
        );
        let m = rmlrs_parser::parse_mapping(&mapping, None).unwrap();
        let out: BTreeSet<String> = run(&m, &Options::default())
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        let expected: BTreeSet<String> = [
            r#"<http://example.com/resource/student_10> <http://xmlns.com/foaf/0.1/name> "Venus Williams" ."#,
            r#"<http://example.com/resource/student_20> <http://xmlns.com/foaf/0.1/name> "Demi Moore" ."#,
            r#"<http://example.com/resource/sport_100> <http://www.w3.org/2000/01/rdf-schema#label> "Tennis" ."#,
            "<http://example.com/resource/student_10> <http://example.com/ontology/practises> <http://example.com/resource/sport_100> .",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect();
        assert_eq!(out, expected);
    }

    /// A triples map over SPARQL solutions: `?s` names the subject, `?o`
    /// the object.
    fn sparql_mapping(source: &str, rf: &str) -> Mapping {
        rmlrs_parser::parse_mapping(
            &format!(
                r#"@prefix rml: <http://w3id.org/rml/> . @prefix ex: <http://example.com/> .
                   @prefix sd: <http://www.w3.org/ns/sparql-service-description#> .
                   @prefix formats: <http://www.w3.org/ns/formats/> .
                   ex:TM rml:logicalSource [ rml:source {source} ; rml:referenceFormulation {rf} ;
                       rml:iterator "SELECT ?s ?o WHERE {{ ?s ex:p ?o }}" ] ;
                     rml:subjectMap [ rml:reference "s" ] ;
                     rml:predicateObjectMap [ rml:predicate ex:p ; rml:objectMap [ rml:reference "o" ] ] ."#
            ),
            None,
        )
        .unwrap()
    }

    #[test]
    fn sparql_needs_an_endpoint_and_a_results_format() {
        // A query over an RDF file would need a SPARQL engine.
        let m = sparql_mapping(
            r#"[ a rml:RelativePathSource ; rml:path "data.nt" ]"#,
            "formats:SPARQL_Results_CSV",
        );
        assert!(matches!(
            run(&m, &Options::default()),
            Err(Error::Unsupported(_))
        ));
        let endpoint = "[ a sd:Service ; sd:endpoint <http://localhost:1/sparql> ]";
        let m = sparql_mapping(endpoint, "rml:JSONPath");
        assert!(matches!(
            run(&m, &Options::default()),
            Err(Error::Mapping(_))
        ));
        let m = sparql_mapping(endpoint, "formats:SPARQL_Results_XML");
        assert!(matches!(
            run(&m, &Options::default()),
            Err(Error::Unsupported(_))
        ));
    }

    #[cfg(not(feature = "http"))]
    #[test]
    fn sparql_endpoints_need_the_http_feature() {
        let m = sparql_mapping(
            "[ a sd:Service ; sd:endpoint <http://localhost:1/sparql> ]",
            "formats:SPARQL_Results_CSV",
        );
        assert!(matches!(
            run(&m, &Options::default()),
            Err(Error::Unsupported(_))
        ));
    }

    #[cfg(feature = "http")]
    #[test]
    fn sparql_endpoints_are_queried() {
        let csv = "s,o\r\nhttp://e/a,1\r\nhttp://e/b,\r\n";
        let url = serve_once("200 OK", csv);
        let m = sparql_mapping(
            &format!("[ a sd:Service ; sd:endpoint <{url}> ]"),
            "formats:SPARQL_Results_CSV",
        );
        let out: Vec<String> = run(&m, &Options::default())
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        // An unbound variable gives no value, so http://e/b has no triple.
        assert_eq!(out, ["<http://e/a> <http://example.com/p> \"1\" ."]);
    }

    #[cfg(feature = "http")]
    #[test]
    fn web_of_things_sources_are_fetched() {
        let url = serve_once("200 OK", r#"[{"a": 1}, {"a": 2}, {"a": 3}]"#);
        let m = remote_mapping(&format!(
            r#"td:hasPropertyAffordance [ td:hasForm [ hctl:hasTarget "{url}" ; hctl:forContentType "application/json" ] ]"#
        ));
        assert_eq!(run(&m, &Options::default()).unwrap().len(), 3);
        let url = serve_once("500 Internal Server Error", "");
        let m = remote_mapping(&format!("dcat:downloadURL <{url}>"));
        assert!(matches!(
            run(&m, &Options::default()),
            Err(Error::Source(_))
        ));
    }

    #[cfg(feature = "http")]
    #[test]
    fn remote_csvw_urls_are_fetched() {
        let url = serve_once("200 OK", "ID;Name\n10;Venus, W\n");
        let m = people(
            &format!(
                r#"[ a csvw:Table ; csvw:url "{url}" ; csvw:dialect [ csvw:delimiter ";" ] ]"#
            ),
            "CSV",
        );
        let m = rmlrs_parser::parse_mapping(&m, None).unwrap();
        assert_eq!(
            names(&run(&m, &Options::default()).unwrap()),
            pairs(&[("10", "Venus, W")])
        );
    }
}
