// SPDX-License-Identifier: Apache-2.0
//! `rmlrs run mapping.ttl -o out.nq`

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "rmlrs", version, about = "Run RML mappings and write RDF")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run a mapping and write N-Quads.
    Run {
        /// The mapping document (Turtle).
        mapping: PathBuf,
        /// Output file; standard output if omitted.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Base IRI for relative IRIs.
        #[arg(long, default_value = "http://example.com/")]
        base_iri: String,
        /// The database an R2RML mapping reads its logical tables from, as a
        /// JDBC connection string (e.g. jdbc:postgresql://localhost:5432/db).
        #[arg(long)]
        db_url: Option<String>,
        /// User for --db-url.
        #[arg(long, requires = "db_url")]
        db_user: Option<String>,
        /// Password for --db-url; read from RMLRS_DB_PASSWORD when omitted.
        #[arg(long, requires = "db_url")]
        db_password: Option<String>,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Run {
            mapping,
            output,
            base_iri,
            db_url,
            db_user,
            db_password,
        } => {
            let database = db_url.map(|dsn| rmlrs::DatabaseSource {
                dsn,
                driver: None,
                username: db_user,
                password: db_password.or_else(|| std::env::var("RMLRS_DB_PASSWORD").ok()),
            });
            match execute(&mapping, output, &base_iri, database) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("rmlrs: {e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn execute(
    mapping: &PathBuf,
    output: Option<PathBuf>,
    base_iri: &str,
    database: Option<rmlrs::DatabaseSource>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut out: Box<dyn Write> = match output {
        Some(p) => Box::new(BufWriter::with_capacity(1 << 16, File::create(p)?)),
        None => Box::new(BufWriter::with_capacity(1 << 16, std::io::stdout().lock())),
    };
    let text = std::fs::read_to_string(mapping)
        .map_err(|e| rmlrs::Error::Source(format!("{}: {e}", mapping.display())))?;
    let options = rmlrs::Options {
        base_iri: base_iri.to_owned(),
        mapping_dir: mapping.parent().map(PathBuf::from).unwrap_or_default(),
        working_dir: std::env::current_dir()?,
        database,
        ..Default::default()
    };
    rmlrs::run_with_targets(&rmlrs::parse_mapping(&text, None)?, &options, &mut |q| {
        writeln!(out, "{q}").map_err(|e| rmlrs::Error::Source(format!("writing output: {e}")))
    })?;
    out.flush()?;
    Ok(())
}
