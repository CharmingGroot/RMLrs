// SPDX-License-Identifier: Apache-2.0
//! Runs the KG-Construct RML test suites against RMLrs.
//!
//! Results are written in the format of the official implementation report
//! (`testcase_id,status,notes`), one file per suite, and summarised per
//! module. `--check` compares with a baseline and fails on regressions.
//!
//! A test case that expects an error passes only when RMLrs reports a
//! mapping, data, source or syntax error. "Not supported" never counts.

mod archive;
mod csv;
mod databases;
mod iso;
mod json_ld;
mod rdf_json;
mod rdf_xml;
#[cfg(test)]
mod round_trip;

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(about = "Run the RML conformance test suites")]
struct Args {
    /// Directory holding the suites as subdirectories (rml-core, rml-io, ...).
    #[arg(long, default_value = "conformance/suites")]
    suites: PathBuf,
    /// Only run these suites (comma separated, e.g. rml-core,rml-io).
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,
    /// Where to write per-suite result CSVs.
    #[arg(long, default_value = "conformance/results")]
    out: PathBuf,
    /// Fail if a case that passes in this baseline no longer passes.
    #[arg(long)]
    check: Option<PathBuf>,
    /// Write the current results as the new baseline.
    #[arg(long)]
    write_baseline: Option<PathBuf>,
    /// Start database containers (Docker) and run the SQL cases.
    #[arg(long)]
    databases: bool,
    /// Print each case's status and notes.
    #[arg(short, long)]
    verbose: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Passed,
    Failed,
    /// Part of the report format; used once cases can be ruled out of scope.
    #[allow(dead_code)]
    Inapplicable,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Status::Passed => "passed",
            Status::Failed => "failed",
            Status::Inapplicable => "inapplicable",
        }
    }
}

#[derive(Debug)]
struct Outcome {
    suite: String,
    id: String,
    status: Status,
    notes: String,
}

const SUITES: [&str; 9] = [
    "rml-core",
    "rml-io",
    "rml-io-registry",
    "rml-fnml",
    "rml-cc",
    "rml-lv",
    "rml-star",
    // Legacy vocabularies: the old RML test cases (RMLio) and the W3C R2RML
    // test cases (from kg-construct/r2rml-test-cases-support).
    "rml-test-cases",
    "r2rml",
];

fn main() -> ExitCode {
    let args = Args::parse();
    let mut dbs = if args.databases {
        match databases::Databases::start() {
            Ok(d) => Some(d),
            Err(e) => {
                eprintln!("{e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        None
    };
    let mut outcomes = Vec::new();
    for suite in SUITES {
        if !args.only.is_empty() && !args.only.iter().any(|o| o == suite) {
            continue;
        }
        let dir = match suite {
            "r2rml" | "rml-test-cases" => args.suites.join(suite),
            _ => args.suites.join(suite).join("test-cases"),
        };
        match run_suite(suite, &dir, dbs.as_mut()) {
            Ok(o) => outcomes.extend(o),
            Err(e) => {
                eprintln!("{suite}: {e}");
                return ExitCode::FAILURE;
            }
        }
    }

    if let Some(d) = dbs.as_mut() {
        d.stop();
    }
    if args.verbose {
        for o in &outcomes {
            println!("{:<28} {:<12} {}", o.id, o.status.as_str(), o.notes);
        }
    }
    annotate_known_issues(
        &args.suites.join("..").join("known-issues.csv"),
        &mut outcomes,
    );
    if let Err(e) = write_results(&args.out, &outcomes) {
        eprintln!("writing results: {e}");
        return ExitCode::FAILURE;
    }
    print_summary(&outcomes);

    if let Some(path) = &args.write_baseline
        && let Err(e) = write_baseline(path, &outcomes)
    {
        eprintln!("writing baseline: {e}");
        return ExitCode::FAILURE;
    }
    if let Some(path) = &args.check {
        match check_baseline(path, &outcomes) {
            Ok(0) => println!("baseline: no regressions"),
            Ok(n) => {
                eprintln!("baseline: {n} regression(s)");
                return ExitCode::FAILURE;
            }
            Err(e) => {
                eprintln!("reading baseline: {e}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn run_suite(
    suite: &str,
    dir: &Path,
    mut dbs: Option<&mut databases::Databases>,
) -> Result<Vec<Outcome>, String> {
    let cases = match suite {
        "r2rml" => r2rml_cases(dir)?,
        "rml-test-cases" => old_rml_cases(dir)?,
        _ => kg_construct_cases(suite, dir)?,
    };
    let mut out = Vec::new();
    for (id, case) in cases {
        let (status, notes) =
            match catch_unwind(AssertUnwindSafe(|| run_case(&case, dbs.as_deref_mut()))) {
                Ok(r) => r,
                Err(_) => (Status::Failed, "panic".to_owned()),
            };
        out.push(Outcome {
            suite: suite.to_owned(),
            id,
            status,
            notes,
        });
    }
    Ok(out)
}

/// The rows of a `metadata.csv`, as a column lookup per row.
fn metadata(dir: &Path) -> Result<Vec<HashMap<String, String>>, String> {
    let meta = fs::read_to_string(dir.join("metadata.csv"))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let rows = csv::parse(&meta);
    let Some((header, rows)) = rows.split_first() else {
        return Ok(Vec::new());
    };
    Ok(rows
        .iter()
        .map(|row| {
            header
                .iter()
                .cloned()
                .zip(row.iter().cloned())
                .collect::<HashMap<_, _>>()
        })
        .collect())
}

/// The KG-Construct suites (rml-core, rml-io, ...).
fn kg_construct_cases(suite: &str, dir: &Path) -> Result<Vec<(String, Case)>, String> {
    let mut out = Vec::new();
    for row in metadata(dir)? {
        let get = |name: &str| row.get(name).cloned().unwrap_or_default();
        let id = get("ID");
        if id.is_empty() {
            continue;
        }
        let case = Case {
            dir: dir.join(&id),
            base_iri: match get("base_iri") {
                // rml-cc's metadata has no base IRI column, but its expected
                // outputs resolve relative IRIs against this one.
                b if b.is_empty() && suite == "rml-cc" => "http://example.com/base/".to_owned(),
                b => b,
            },
            mapping: get("mapping"),
            title: get("title"),
            sql: get("input_format1") == "application/sql",
            fixture: dir.join(&id).join("resource.sql"),
            processor_db: false,
            expects_error: get("error") == "true",
            inputs: (1..=3)
                .map(|i| get(&format!("input{i}")))
                .filter(|s| !s.is_empty())
                .collect(),
            outputs: (1..=3)
                .map(|i| get(&format!("output{i}")))
                .filter(|s| !s.is_empty())
                .collect(),
            output_formats: (1..=3)
                .map(|i| get(&format!("output_format{i}")))
                .filter(|s| !s.is_empty())
                .collect(),
        };
        out.push((id, case));
    }
    Ok(out)
}

/// The old RML test cases (legacy vocabulary): `mapping.ttl`, and
/// `output.nq` unless an error is expected.
fn old_rml_cases(dir: &Path) -> Result<Vec<(String, Case)>, String> {
    let mut out = Vec::new();
    for row in metadata(dir)? {
        let get = |name: &str| row.get(name).cloned().unwrap_or_default();
        let id = get("RML id");
        if id.is_empty() {
            continue;
        }
        // The database is in the id ("RMLTC0009a-PostgreSQL"); the format
        // column says only "table".
        let format = id.rsplit('-').next().unwrap_or_default().to_owned();
        let case_dir = dir.join("test-cases").join(&id);
        let has_output = case_dir.join("output.nq").exists();
        let case = Case {
            base_iri: "http://example.com/base/".to_owned(),
            mapping: "mapping.ttl".to_owned(),
            // databases::db_of reads the database from the title.
            title: format!("{format}: {}", get("title")),
            sql: matches!(format.as_str(), "MySQL" | "PostgreSQL" | "SQLServer"),
            fixture: case_dir.join("resource.sql"),
            processor_db: false,
            expects_error: get("error expected?") == "true" || !has_output,
            inputs: (1..=3)
                .map(|i| get(&format!("input file {i}")))
                .filter(|s| !s.is_empty())
                .collect(),
            outputs: if has_output {
                vec!["output.nq".to_owned()]
            } else {
                Vec::new()
            },
            output_formats: vec!["application/n-quads".to_owned()],
            dir: case_dir,
        };
        out.push((id, case));
    }
    Ok(out)
}

/// The W3C R2RML test cases, from their manifest. Each runs against
/// PostgreSQL, loaded with the test's database script; the connection is
/// given to the processor (`Options::database`), as R2RML has it.
fn r2rml_cases(dir: &Path) -> Result<Vec<(String, Case)>, String> {
    let text = fs::read_to_string(dir.join("manifest.ttl"))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    let triples = rmlrs::turtle::parse(&text, None).map_err(|e| e.to_string())?;
    const T: &str = "http://purl.org/NET/rdb2rdf-test#";
    const DC: &str = "http://purl.org/dc/terms/";
    let value = |s: &rmlrs::Term, p: &str| {
        triples
            .iter()
            .find(|t| &t.subject == s && t.predicate.as_iri() == Some(p))
            .map(|t| t.object.clone())
    };
    let text_of = |t: Option<rmlrs::Term>| match t {
        Some(rmlrs::Term::Literal(l)) => l.value,
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let r2rml = rmlrs::Term::iri(format!("{T}R2RML"));
    let mut out = Vec::new();
    for t in &triples {
        if t.predicate.as_iri() != Some(rmlrs_rdf_type()) || t.object != r2rml {
            continue;
        }
        let case = &t.subject;
        let id = text_of(value(case, &format!("{DC}identifier")));
        let database = value(case, &format!("{T}database"));
        let mut script = database
            .map(|d| text_of(value(&d, &format!("{T}sqlScriptFile"))))
            .unwrap_or_default();
        // As the suite's own test.py does: d016.sql uses VARBINARY.
        if script == "d016.sql" {
            script = "d016-postgresql.sql".to_owned();
        }
        let has_output = text_of(value(case, &format!("{T}hasExpectedOutput"))) == "true";
        let output = text_of(value(case, &format!("{T}output")));
        let c = Case {
            dir: dir.join(&id),
            base_iri: "http://example.com/base/".to_owned(),
            mapping: text_of(value(case, &format!("{T}mappingDocument"))),
            title: format!("PostgreSQL: {id}"),
            sql: true,
            fixture: dir.join("databases").join(script),
            processor_db: true,
            expects_error: !has_output,
            inputs: Vec::new(),
            outputs: if has_output { vec![output] } else { Vec::new() },
            output_formats: vec!["application/n-quads".to_owned()],
        };
        out.push((id, c));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn rmlrs_rdf_type() -> &'static str {
    "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
}

struct Case {
    dir: PathBuf,
    base_iri: String,
    mapping: String,
    title: String,
    sql: bool,
    /// The database script loaded before an SQL case.
    fixture: PathBuf,
    /// The connection is given to the processor (R2RML) instead of being
    /// written into the mapping.
    processor_db: bool,
    expects_error: bool,
    /// The input files the case declares, in its directory.
    inputs: Vec<String>,
    outputs: Vec<String>,
    output_formats: Vec<String>,
}

/// Where the specifications publish their test data. A remote source under
/// it is the case's own input file of the same name.
const RESOURCES: [&str; 2] = [
    "http://w3id.org/rml/resources/",
    "https://w3id.org/rml/resources/",
];

/// The remote documents of a mapping, read from the case's input files
/// instead of the network: the runner never goes online, so a source it
/// cannot map to a local file is not supported here.
fn remote_documents(
    case: &Case,
    work: &Path,
    mapping: &rmlrs::Mapping,
) -> rmlrs::Result<HashMap<String, String>> {
    if let Some(endpoint) = mapping.sparql_endpoints().first() {
        return Err(rmlrs::Error::Unsupported(format!(
            "SPARQL endpoint {endpoint} (the conformance runner does not use the network)"
        )));
    }
    let mut docs = HashMap::new();
    for url in mapping.remote_urls() {
        let name = RESOURCES
            .iter()
            .find_map(|r| url.strip_prefix(r))
            .and_then(|path| path.rsplit('/').next())
            .filter(|name| case.inputs.iter().any(|i| i == name));
        let Some(name) = name else {
            return Err(rmlrs::Error::Unsupported(format!(
                "remote source {url} (the conformance runner does not use the network)"
            )));
        };
        let text = fs::read_to_string(work.join(name))
            .map_err(|e| rmlrs::Error::Source(format!("{name}: {e}")))?;
        docs.insert(url.to_owned(), text);
    }
    Ok(docs)
}

fn run_case(case: &Case, dbs: Option<&mut databases::Databases>) -> (Status, String) {
    if !case.expects_error
        && case
            .output_formats
            .iter()
            // N-Triples(-star) outputs are compared as N-Quads in the default graph.
            .any(|f| f != "application/n-quads" && f != "application/n-triples")
    {
        return (
            Status::Failed,
            format!("not supported: output format {:?}", case.output_formats),
        );
    }
    // Run in a scratch copy: logical targets write files next to the
    // expected ones, and must not overwrite them.
    let work = match scratch_copy(&case.dir) {
        Ok(w) => w,
        Err(e) => return (Status::Failed, format!("copying the case: {e}")),
    };
    let mut default_output = Vec::new();
    let mut text = match fs::read_to_string(work.join(&case.mapping)) {
        Ok(t) => t,
        Err(e) => return (Status::Failed, format!("reading mapping: {e}")),
    };
    let mut database = None;
    if case.sql {
        let Some(db) = databases::db_of(&case.title, &text) else {
            return (Status::Failed, "not supported: unknown database".into());
        };
        let Some(dbs) = dbs else {
            return (
                Status::Failed,
                format!("not supported: {db:?} database (run with --databases)"),
            );
        };
        let fixture = fs::read_to_string(&case.fixture).unwrap_or_default();
        match dbs.prepare(db, &fixture) {
            Ok(dsn) if case.processor_db => {
                database = Some(rmlrs::DatabaseSource {
                    dsn,
                    driver: None,
                    username: Some("postgres".into()),
                    password: None,
                });
            }
            Ok(dsn) => text = text.replace("CONNECTIONDSN", &dsn),
            Err(e) => return (Status::Failed, e),
        }
    }
    let result = rmlrs::parse_mapping(&text, None).and_then(|mapping| {
        let base = if case.base_iri.is_empty() {
            "http://example.com/"
        } else {
            case.base_iri.as_str()
        };
        let options = rmlrs::Options {
            base_iri: base.to_owned(),
            mapping_dir: work.clone(),
            working_dir: work.clone(),
            in_memory: remote_documents(case, &work, &mapping)?,
            database,
            ..Default::default()
        };
        rmlrs::run_with_targets(&mapping, &options, &mut |q| {
            default_output.push(q);
            Ok(())
        })
    });

    let verdict = match (case.expects_error, result) {
        (true, Err(e)) if e.is_spec_error() => (Status::Passed, e.to_string()),
        (true, Err(e)) => (Status::Failed, e.to_string()),
        (true, Ok(())) => (Status::Failed, "expected an error, got output".into()),
        (false, Err(e)) => (Status::Failed, e.to_string()),
        (false, Ok(())) => compare_outputs(case, &work, &default_output),
    };
    fs::remove_dir_all(&work).ok();
    verdict
}

/// The first expected output is the default output; the others are files
/// written by logical targets, found under the same name in the scratch copy.
fn compare_outputs(case: &Case, work: &Path, default_output: &[rmlrs::Quad]) -> (Status, String) {
    for (i, name) in case.outputs.iter().enumerate() {
        let expected = match read_rdf(&case.dir.join(name)) {
            Ok(q) => q,
            Err(e) => return (Status::Failed, format!("reading expected {name}: {e}")),
        };
        let actual = if i == 0 {
            default_output.to_vec()
        } else {
            match read_rdf(&work.join(name)) {
                Ok(q) => q,
                Err(e) => return (Status::Failed, format!("reading produced {name}: {e}")),
            }
        };
        if !iso::isomorphic(&actual, &expected) {
            // RMLRS_DUMP=<dir> keeps the produced output of failing cases.
            if let Ok(dump) = std::env::var("RMLRS_DUMP") {
                let id = case
                    .dir
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let _ = fs::create_dir_all(&dump);
                let _ = fs::write(
                    Path::new(&dump).join(format!("{id}.{name}")),
                    rmlrs::to_nquads(&actual),
                );
            }
            return (
                Status::Failed,
                format!(
                    "{name} differs: {} quads, expected {}",
                    actual.len(),
                    expected.len()
                ),
            );
        }
    }
    (Status::Passed, String::new())
}

/// Reads an RDF file by its extension (of the archive member, for
/// compressed files). UTF-16 files (with a byte order mark) are decoded first.
fn read_rdf(path: &Path) -> Result<Vec<rmlrs::Quad>, String> {
    let (bytes, name) = archive::unpack(path)?;
    let text = decode(&bytes)?;
    let ext = Path::new(&name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    match ext {
        "nq" | "nt" => rmlrs::nquads::parse(&text).map_err(|e| e.to_string()),
        "ttl" | "n3" => Ok(rmlrs::turtle::parse(&text, None)
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(|t| rmlrs::Quad {
                subject: t.subject,
                predicate: t.predicate,
                object: t.object,
                graph: None,
            })
            .collect()),
        "rdfjson" => rdf_json::parse(&text),
        "jsonld" => json_ld::parse(&text),
        "rdfxml" | "rdf" => rdf_xml::parse(&text),
        other => Err(format!("cannot compare .{other} files")),
    }
}

fn decode(bytes: &[u8]) -> Result<String, String> {
    let utf16 = |be: bool, b: &[u8]| {
        let units: Vec<u16> = b
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&c| {
                if be {
                    u16::from_be_bytes(c)
                } else {
                    u16::from_le_bytes(c)
                }
            })
            .collect();
        String::from_utf16(&units).map_err(|e| e.to_string())
    };
    match bytes {
        [0xFF, 0xFE, rest @ ..] => utf16(false, rest),
        [0xFE, 0xFF, rest @ ..] => utf16(true, rest),
        [0xEF, 0xBB, 0xBF, rest @ ..] => {
            String::from_utf8(rest.to_vec()).map_err(|e| e.to_string())
        }
        _ => String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string()),
    }
}

fn scratch_copy(dir: &Path) -> std::io::Result<PathBuf> {
    let name = dir.file_name().unwrap_or_default();
    let work = std::env::temp_dir()
        .join(format!("rmlrs-conformance-{}", std::process::id()))
        .join(name);
    if work.exists() {
        fs::remove_dir_all(&work)?;
    }
    copy_dir(dir, &work)?;
    Ok(work)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// Appends the note of a known test-suite problem to a failing case. The
/// status is not changed: a known issue still counts as failed.
/// The key of an outcome in the baseline and the known issues: the case id,
/// prefixed with the suite for the legacy suites, whose ids repeat those of
/// rml-core ("RMLTC0007h-JSON").
fn key(o: &Outcome) -> String {
    match o.suite.as_str() {
        "rml-test-cases" | "r2rml" => format!("{}/{}", o.suite, o.id),
        _ => o.id.clone(),
    }
}

fn annotate_known_issues(path: &Path, outcomes: &mut [Outcome]) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let notes: BTreeMap<String, String> = csv::parse(&text)
        .into_iter()
        .skip(1)
        .filter_map(|r| Some((r.first()?.clone(), r.get(1)?.clone())))
        .collect();
    for o in outcomes.iter_mut().filter(|o| o.status == Status::Failed) {
        if let Some(n) = notes.get(&key(o)) {
            o.notes = format!("{n}; {}", o.notes);
        }
    }
}

fn write_results(dir: &Path, outcomes: &[Outcome]) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let mut by_suite: BTreeMap<&str, Vec<&Outcome>> = BTreeMap::new();
    for o in outcomes {
        by_suite.entry(&o.suite).or_default().push(o);
    }
    for (suite, os) in by_suite {
        let mut s = String::from("testcase_id,status,notes\n");
        for o in os {
            s.push_str(&format!(
                "{},{},{}\n",
                o.id,
                o.status.as_str(),
                csv::quote(&o.notes)
            ));
        }
        fs::write(dir.join(format!("{suite}.csv")), s)?;
    }
    Ok(())
}

fn print_summary(outcomes: &[Outcome]) {
    let mut by_suite: BTreeMap<&str, (usize, usize)> = BTreeMap::new();
    for o in outcomes {
        let e = by_suite.entry(&o.suite).or_default();
        e.1 += 1;
        if o.status == Status::Passed {
            e.0 += 1;
        }
    }
    println!("{:<18} {:>6} {:>6}", "suite", "passed", "total");
    let (mut p, mut t) = (0, 0);
    for (suite, (passed, total)) in by_suite {
        println!("{suite:<18} {passed:>6} {total:>6}");
        p += passed;
        t += total;
    }
    println!("{:<18} {p:>6} {t:>6}", "all");
}

fn write_baseline(path: &Path, outcomes: &[Outcome]) -> std::io::Result<()> {
    let mut s = String::from("testcase_id,status\n");
    let mut rows: Vec<_> = outcomes
        .iter()
        .map(|o| format!("{},{}", key(o), o.status.as_str()))
        .collect();
    rows.sort();
    for r in rows {
        s.push_str(&r);
        s.push('\n');
    }
    fs::write(path, s)
}

/// Number of cases that pass in the baseline but not now.
fn check_baseline(path: &Path, outcomes: &[Outcome]) -> std::io::Result<usize> {
    let text = fs::read_to_string(path)?;
    let now: BTreeMap<String, Status> = outcomes.iter().map(|o| (key(o), o.status)).collect();
    let skipped: std::collections::BTreeSet<String> = outcomes
        .iter()
        .filter(|o| o.notes.contains("(run with --databases)"))
        .map(key)
        .collect();
    let mut regressions = 0;
    for row in csv::parse(&text).into_iter().skip(1) {
        let (Some(id), Some(status)) = (row.first(), row.get(1)) else {
            continue;
        };
        if status == "passed" {
            if skipped.contains(id) {
                continue; // SQL case in a run without --databases
            }
            match now.get(id) {
                Some(Status::Passed) => {}
                Some(_) => {
                    eprintln!("regression: {id}");
                    regressions += 1;
                }
                None => {} // suite not run this time
            }
        }
    }
    Ok(regressions)
}
