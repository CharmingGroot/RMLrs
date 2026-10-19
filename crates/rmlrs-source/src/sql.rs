// SPDX-License-Identifier: Apache-2.0
//! SQL sources (RML-IO-Registry, SQL reference formulation).
//!
//! One iteration is one row of the query result; a reference is a column
//! name of that result. A reference matches the column name exactly (in
//! double quotes, the quotes are removed first). Failing that, an unquoted
//! reference follows the database: PostgreSQL folds it to lower case, MySQL
//! and SQL Server compare without case. (RMLIOREGTC0005a needs the exact
//! match, 0005k the folding.) A table name given as iterator is SQL text,
//! as `rr:tableName` is in R2RML.
//!
//! Values get the natural RDF datatypes of R2RML: integers `xsd:integer`,
//! NUMERIC/DECIMAL `xsd:decimal`, FLOAT/REAL `xsd:double`, BOOLEAN
//! `xsd:boolean`, DATE/TIME/TIMESTAMP `xsd:date`/`xsd:time`/`xsd:dateTime`,
//! binary `xsd:hexBinary`, everything else `xsd:string`. NULL gives no value.
//!
//! PostgreSQL results are read through a cursor, a batch of rows at a time.
//! MySQL and SQL Server results are still received whole before the first
//! row is mapped.

use std::sync::Arc;

use rmlrs_model::{
    DatabaseSource, Error, Result, XSD_BOOLEAN, XSD_DOUBLE, XSD_INTEGER, XSD_STRING,
};

use crate::{Iteration, Iterations, NaturalValue, canonical_double};

const XSD_DECIMAL: &str = "http://www.w3.org/2001/XMLSchema#decimal";
const XSD_DATE: &str = "http://www.w3.org/2001/XMLSchema#date";
const XSD_TIME: &str = "http://www.w3.org/2001/XMLSchema#time";
const XSD_DATETIME: &str = "http://www.w3.org/2001/XMLSchema#dateTime";
const XSD_HEXBINARY: &str = "http://www.w3.org/2001/XMLSchema#hexBinary";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Postgres,
    MySql,
    SqlServer,
}

impl Dialect {
    /// The dialect named by a JDBC connection string.
    pub fn of(dsn: &str) -> Result<Dialect> {
        let rest = dsn.strip_prefix("jdbc:").unwrap_or(dsn);
        if rest.starts_with("postgresql:") {
            Ok(Dialect::Postgres)
        } else if rest.starts_with("mysql:") || rest.starts_with("mariadb:") {
            Ok(Dialect::MySql)
        } else if rest.starts_with("sqlserver:") {
            Ok(Dialect::SqlServer)
        } else {
            Err(Error::Unsupported(format!("database connection {dsn:?}")))
        }
    }
}

/// One row of a query result.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    columns: Arc<Columns>,
    values: Vec<Option<NaturalValue>>,
}

#[derive(Debug, PartialEq)]
struct Columns {
    names: Vec<String>,
    dialect: Dialect,
}

impl Row {
    /// The value of the column a reference names; `Ok(None)` for SQL NULL.
    /// A reference to a column that does not exist is a data error.
    pub fn get(&self, reference: &str) -> Result<Option<&NaturalValue>> {
        let i = self.columns.position(reference)?;
        Ok(self.values[i].as_ref())
    }
}

impl Columns {
    fn position(&self, reference: &str) -> Result<usize> {
        let name = delimited(reference).unwrap_or_else(|| reference.to_owned());
        let exact = self.names.iter().position(|n| *n == name);
        let found = match (exact, self.dialect) {
            (Some(i), _) => Some(i),
            (None, Dialect::MySql | Dialect::SqlServer) if delimited(reference).is_none() => self
                .names
                .iter()
                .position(|n| n.eq_ignore_ascii_case(&name)),
            // PostgreSQL folds unquoted identifiers to lower case: `ID`
            // names the column created as `ID` (exact) or as `id`.
            (None, Dialect::Postgres) if delimited(reference).is_none() => {
                let folded = name.to_lowercase();
                self.names.iter().position(|n| *n == folded)
            }
            (None, _) => None,
        };
        found.ok_or_else(|| {
            Error::Data(format!(
                "no column {reference} in the result ({})",
                self.names.join(", ")
            ))
        })
    }
}

/// `"Name"` → `Name` (with `""` as an escaped quote); `None` if not quoted.
fn delimited(reference: &str) -> Option<String> {
    let inner = reference.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\"\"", "\""))
}

/// The query a logical source stands for.
pub fn query_for(table_or_query: &str, is_table: bool) -> String {
    if is_table {
        format!("SELECT * FROM {table_or_query}")
    } else {
        table_or_query.to_owned()
    }
}

/// Runs the query and returns its rows as iterations.
pub fn open(db: &DatabaseSource, query: &str) -> Result<Iterations<'static>> {
    match Dialect::of(&db.dsn)? {
        #[cfg(feature = "postgres")]
        Dialect::Postgres => postgres_rows(db, query),
        #[cfg(feature = "mysql")]
        Dialect::MySql => mysql_rows(db, query),
        #[cfg(feature = "mssql")]
        Dialect::SqlServer => mssql_rows(db, query),
        #[allow(unreachable_patterns)] // all database features on
        other => Err(Error::Unsupported(format!(
            "{other:?} sources (not built with that database feature)"
        ))),
    }
}

/// `jdbc:<kind>://host[:port]/database[?...]` → (host, port, database).
/// SQL Server's form is `jdbc:sqlserver://host[:port];databaseName=db;...`.
fn host_port_db(dsn: &str, default_port: u16) -> Result<(String, u16, String)> {
    let bad = || Error::Source(format!("cannot read connection string {dsn:?}"));
    let rest = dsn.split_once("://").ok_or_else(bad)?.1;
    if let Some((authority, params)) = rest.split_once(';') {
        let db = params
            .split(';')
            .find_map(|kv| {
                kv.split_once('=')
                    .filter(|(k, _)| k.eq_ignore_ascii_case("databaseName"))
                    .map(|(_, v)| v)
            })
            .unwrap_or_default()
            .to_owned();
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => (h.to_owned(), p.parse().map_err(|_| bad())?),
            None => (authority.to_owned(), default_port),
        };
        return Ok((host, port, db));
    }
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let db = path.split(['?', ';']).next().unwrap_or_default().to_owned();
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => (h.to_owned(), p.parse().map_err(|_| bad())?),
        None => (authority.to_owned(), default_port),
    };
    Ok((host, port, db))
}

/// Rows fetched from a PostgreSQL cursor per round trip: the most rows of
/// a result that are in memory at once.
#[cfg(feature = "postgres")]
const PG_FETCH_ROWS: usize = 1000;

#[cfg(feature = "postgres")]
fn postgres_rows(db: &DatabaseSource, query: &str) -> Result<Iterations<'static>> {
    postgres_cursor(db, query, PG_FETCH_ROWS)
}

/// Reads the result through a cursor, `fetch_rows` rows at a time, instead
/// of receiving the whole result before the first row is mapped.
#[cfg(feature = "postgres")]
fn postgres_cursor(
    db: &DatabaseSource,
    query: &str,
    fetch_rows: usize,
) -> Result<Iterations<'static>> {
    let (host, port, dbname) = host_port_db(&db.dsn, 5432)?;
    let mut config = postgres::Config::new();
    config.host(&host).port(port);
    if !dbname.is_empty() {
        config.dbname(&dbname);
    }
    if let Some(u) = &db.username {
        config.user(u);
    }
    if let Some(p) = db.password.as_deref().filter(|p| !p.is_empty()) {
        config.password(p);
    }
    let err = |e: postgres::Error| Error::Source(format!("PostgreSQL: {}", pg_message(&e)));
    let mut client = config.connect(postgres::NoTls).map_err(err)?;
    let data = |e: postgres::Error| Error::Data(format!("PostgreSQL: {}", pg_message(&e)));
    // The prepared statement gives the column types; the cursor, fetched
    // with simple queries, gives every value in PostgreSQL's text form,
    // which the types then interpret.
    let stmt = client.prepare(query).map_err(data)?;
    let types: Vec<String> = stmt
        .columns()
        .iter()
        .map(|c| c.type_().name().to_owned())
        .collect();
    let names: Vec<String> = stmt.columns().iter().map(|c| c.name().to_owned()).collect();
    let columns = Arc::new(Columns {
        names,
        dialect: Dialect::Postgres,
    });
    // A cursor lives in a transaction. The line break ends a trailing
    // `--` comment of the query.
    client.batch_execute("BEGIN").map_err(err)?;
    client
        .batch_execute(&format!(
            "DECLARE rmlrs_rows NO SCROLL CURSOR FOR {query}\n"
        ))
        .map_err(data)?;
    Ok(Box::new(PgCursor {
        client,
        fetch: format!("FETCH FORWARD {fetch_rows} FROM rmlrs_rows"),
        fetch_rows,
        columns,
        types,
        batch: Vec::new().into_iter(),
        done: false,
    }))
}

/// The rows of a PostgreSQL cursor, one fetched batch in memory at a time.
#[cfg(feature = "postgres")]
struct PgCursor {
    client: postgres::Client,
    fetch: String,
    fetch_rows: usize,
    columns: Arc<Columns>,
    types: Vec<String>,
    batch: std::vec::IntoIter<postgres::SimpleQueryMessage>,
    done: bool,
}

#[cfg(feature = "postgres")]
impl Iterator for PgCursor {
    type Item = Result<Iteration>;

    fn next(&mut self) -> Option<Result<Iteration>> {
        loop {
            for m in self.batch.by_ref() {
                if let postgres::SimpleQueryMessage::Row(r) = m {
                    let values = (0..self.types.len())
                        .map(|i| r.get(i).map(|text| postgres_value(text, &self.types[i])))
                        .collect();
                    return Some(Ok(Iteration::Sql(Row {
                        columns: self.columns.clone(),
                        values,
                    })));
                }
            }
            if self.done {
                return None;
            }
            let src = |e: postgres::Error| Error::Source(format!("PostgreSQL: {}", pg_message(&e)));
            let messages = match self.client.simple_query(&self.fetch) {
                Ok(m) => m,
                Err(e) => {
                    self.done = true;
                    return Some(Err(src(e)));
                }
            };
            let rows = messages
                .iter()
                .filter(|m| matches!(m, postgres::SimpleQueryMessage::Row(_)))
                .count();
            if rows < self.fetch_rows {
                // The last batch: ending the transaction closes the cursor.
                self.done = true;
                if let Err(e) = self.client.batch_execute("COMMIT") {
                    return Some(Err(src(e)));
                }
            }
            self.batch = messages.into_iter();
        }
    }
}

/// The server's message ("column \"x\" does not exist") instead of "db error".
#[cfg(feature = "postgres")]
fn pg_message(e: &postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => format!("{} ({})", db.message(), db.code().code()),
        None => e.to_string(),
    }
}

#[cfg(feature = "postgres")]
fn postgres_value(text: &str, type_name: &str) -> NaturalValue {
    let v = |lexical: String, datatype: &'static str| NaturalValue { lexical, datatype };
    match type_name {
        "int2" | "int4" | "int8" => v(canonical_integer(text), XSD_INTEGER),
        "numeric" => v(canonical_decimal(text), XSD_DECIMAL),
        "float4" | "float8" => match text.parse::<f64>() {
            Ok(f) => v(canonical_double(f), XSD_DOUBLE),
            Err(_) => v(text.to_owned(), XSD_DOUBLE),
        },
        "bool" => v(
            if text == "t" { "true" } else { "false" }.to_owned(),
            XSD_BOOLEAN,
        ),
        "date" => v(text.to_owned(), XSD_DATE),
        "time" | "timetz" => v(text.to_owned(), XSD_TIME),
        "timestamp" | "timestamptz" => v(text.replacen(' ', "T", 1), XSD_DATETIME),
        "bytea" => v(
            text.trim_start_matches("\\x").to_ascii_uppercase(),
            XSD_HEXBINARY,
        ),
        _ => v(text.to_owned(), XSD_STRING),
    }
}

/// Reads the rows as the server sends them, instead of receiving the whole
/// result before the first row is mapped. The result owns its connection.
#[cfg(feature = "mysql")]
fn mysql_rows(db: &DatabaseSource, query: &str) -> Result<Iterations<'static>> {
    use mysql::prelude::TextQuery;
    let (host, port, dbname) = host_port_db(&db.dsn, 3306)?;
    let opts = mysql::OptsBuilder::new()
        .ip_or_hostname(Some(host))
        .tcp_port(port)
        .db_name(Some(dbname).filter(|d| !d.is_empty()))
        .user(db.username.clone())
        .pass(db.password.clone().filter(|p| !p.is_empty()));
    let conn = mysql::Conn::new(opts).map_err(|e| Error::Source(format!("MySQL: {e}")))?;
    let result = query
        .run(conn)
        .map_err(|e| Error::Data(format!("MySQL: {e}")))?;
    let cols = result.columns();
    // MySQL's BOOLEAN is TINYINT(1): a TINY column of display width 1 is
    // read as a boolean, as the official case RMLIOREGTC0004y expects.
    let meta: Vec<(String, mysql::consts::ColumnType, bool)> = cols
        .as_ref()
        .iter()
        .map(|c| {
            let t = c.column_type();
            let t = if t == mysql::consts::ColumnType::MYSQL_TYPE_TINY && c.column_length() == 1 {
                mysql::consts::ColumnType::MYSQL_TYPE_BIT
            } else {
                t
            };
            (c.name_str().into_owned(), t, c.character_set() == 63)
        })
        .collect();
    let mut names: Vec<String> = meta.iter().map(|m| m.0.clone()).collect();
    if names
        .iter()
        .enumerate()
        .any(|(i, n)| names[..i].contains(n))
    {
        return Err(Error::Data(format!(
            "duplicate column names in the result ({})",
            names.join(", ")
        )));
    }
    let columns = Arc::new(Columns {
        names: std::mem::take(&mut names),
        dialect: Dialect::MySql,
    });
    let types = meta.into_iter().map(|(_, t, binary)| (t, binary)).collect();
    Ok(Box::new(MySqlRows {
        result,
        columns,
        types,
        done: false,
    }))
}

/// The rows of the first result set of a MySQL query, one at a time.
#[cfg(feature = "mysql")]
struct MySqlRows {
    result: mysql::QueryResult<'static, 'static, 'static, mysql::Text>,
    columns: Arc<Columns>,
    types: Vec<(mysql::consts::ColumnType, bool)>,
    done: bool,
}

#[cfg(feature = "mysql")]
impl Iterator for MySqlRows {
    type Item = Result<Iteration>;

    fn next(&mut self) -> Option<Result<Iteration>> {
        if self.done {
            return None;
        }
        // The result's own iterator moves on to the next result set after
        // the first one ends; only the first is the logical source.
        match self.result.next() {
            Some(Ok(row)) => Some(Ok(Iteration::Sql(Row {
                columns: self.columns.clone(),
                values: row
                    .unwrap()
                    .into_iter()
                    .zip(&self.types)
                    .map(|(v, (t, binary))| mysql_value(v, *t, *binary))
                    .collect(),
            }))),
            Some(Err(e)) => {
                self.done = true;
                Some(Err(Error::Source(format!("MySQL: {e}"))))
            }
            None => {
                self.done = true;
                None
            }
        }
    }
}

#[cfg(feature = "mysql")]
fn mysql_value(
    v: mysql::Value,
    t: mysql::consts::ColumnType,
    binary: bool,
) -> Option<NaturalValue> {
    use mysql::consts::ColumnType as C;
    let text = match v {
        mysql::Value::NULL => return None,
        mysql::Value::Bytes(b)
            if binary
                && !matches!(
                    t,
                    C::MYSQL_TYPE_DATE
                        | C::MYSQL_TYPE_DATETIME
                        | C::MYSQL_TYPE_TIMESTAMP
                        | C::MYSQL_TYPE_TIME
                )
                && is_binary_type(t) =>
        {
            return Some(NaturalValue {
                lexical: b.iter().map(|x| format!("{x:02X}")).collect(),
                datatype: XSD_HEXBINARY,
            });
        }
        mysql::Value::Bytes(b) => String::from_utf8_lossy(&b).into_owned(),
        other => other.as_sql(true).trim_matches('\'').to_owned(),
    };
    let v = |lexical: String, datatype: &'static str| Some(NaturalValue { lexical, datatype });
    match t {
        C::MYSQL_TYPE_TINY
        | C::MYSQL_TYPE_SHORT
        | C::MYSQL_TYPE_LONG
        | C::MYSQL_TYPE_LONGLONG
        | C::MYSQL_TYPE_INT24
        | C::MYSQL_TYPE_YEAR => v(canonical_integer(&text), XSD_INTEGER),
        C::MYSQL_TYPE_DECIMAL | C::MYSQL_TYPE_NEWDECIMAL => {
            v(canonical_decimal(&text), XSD_DECIMAL)
        }
        C::MYSQL_TYPE_FLOAT | C::MYSQL_TYPE_DOUBLE => match text.parse::<f64>() {
            Ok(f) => v(canonical_double(f), XSD_DOUBLE),
            Err(_) => v(text, XSD_DOUBLE),
        },
        C::MYSQL_TYPE_DATE => v(text, XSD_DATE),
        C::MYSQL_TYPE_TIME => v(text, XSD_TIME),
        C::MYSQL_TYPE_DATETIME | C::MYSQL_TYPE_TIMESTAMP => {
            v(text.replacen(' ', "T", 1), XSD_DATETIME)
        }
        C::MYSQL_TYPE_BIT => {
            let on = text == "1" || text.as_bytes() == [1];
            v(if on { "true" } else { "false" }.to_owned(), XSD_BOOLEAN)
        }
        _ => v(text, XSD_STRING),
    }
}

#[cfg(feature = "mysql")]
fn is_binary_type(t: mysql::consts::ColumnType) -> bool {
    use mysql::consts::ColumnType as C;
    matches!(
        t,
        C::MYSQL_TYPE_BLOB
            | C::MYSQL_TYPE_TINY_BLOB
            | C::MYSQL_TYPE_MEDIUM_BLOB
            | C::MYSQL_TYPE_LONG_BLOB
            | C::MYSQL_TYPE_VAR_STRING
            | C::MYSQL_TYPE_STRING
            | C::MYSQL_TYPE_VARCHAR
    )
}

#[cfg(feature = "mssql")]
fn mssql_rows(db: &DatabaseSource, query: &str) -> Result<Iterations<'static>> {
    use tokio_util::compat::TokioAsyncWriteCompatExt;
    let (host, port, dbname) = host_port_db(&db.dsn, 1433)?;
    let mut config = tiberius::Config::new();
    config.host(&host);
    config.port(port);
    if !dbname.is_empty() {
        config.database(&dbname);
    }
    config.authentication(tiberius::AuthMethod::sql_server(
        db.username.as_deref().unwrap_or("sa"),
        db.password.as_deref().unwrap_or(""),
    ));
    config.trust_cert();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .map_err(|e| Error::Source(e.to_string()))?;
    let src = |e: tiberius::error::Error| Error::Source(format!("SQL Server: {e}"));
    let (names, rows) = rt.block_on(async {
        let tcp = tokio::net::TcpStream::connect(config.get_addr())
            .await
            .map_err(|e| Error::Source(format!("SQL Server: {e}")))?;
        tcp.set_nodelay(true).ok();
        let mut client = tiberius::Client::connect(config, tcp.compat_write())
            .await
            .map_err(src)?;
        let stream = client
            .simple_query(query)
            .await
            .map_err(|e| Error::Data(format!("SQL Server: {e}")))?;
        let result = stream
            .into_first_result()
            .await
            .map_err(|e| Error::Data(format!("SQL Server: {e}")))?;
        let names: Vec<String> = result
            .first()
            .map(|r| r.columns().iter().map(|c| c.name().to_owned()).collect())
            .unwrap_or_default();
        let rows: Vec<Vec<Option<NaturalValue>>> = result
            .into_iter()
            .map(|r| r.into_iter().map(mssql_value).collect())
            .collect();
        Ok::<_, Error>((names, rows))
    })?;
    if names
        .iter()
        .enumerate()
        .any(|(i, n)| names[..i].contains(n))
    {
        return Err(Error::Data(format!(
            "duplicate column names in the result ({})",
            names.join(", ")
        )));
    }
    let columns = Arc::new(Columns {
        names,
        dialect: Dialect::SqlServer,
    });
    Ok(Box::new(rows.into_iter().map(move |values| {
        Ok(Iteration::Sql(Row {
            columns: columns.clone(),
            values,
        }))
    })))
}

/// A `REAL` as the decimal number it was written as: 70.22 stays 70.22,
/// not the 70.22000122070313 of widening the binary value.
#[cfg(feature = "mssql")]
fn widen_f32(x: f32) -> f64 {
    x.to_string().parse().unwrap_or(f64::from(x))
}

#[cfg(feature = "mssql")]
fn mssql_value(d: tiberius::ColumnData<'static>) -> Option<NaturalValue> {
    use tiberius::time::chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use tiberius::{ColumnData as D, FromSql};
    let v = |lexical: String, datatype: &'static str| Some(NaturalValue { lexical, datatype });
    // chrono's Display forms: "2011-08-23 22:17:00", "2011-08-23", "22:17:00".
    let dt = |x: Option<NaiveDateTime>| x.map(|t| t.to_string().replacen(' ', "T", 1));
    match &d {
        D::U8(Some(x)) => v(x.to_string(), XSD_INTEGER),
        D::I16(Some(x)) => v(x.to_string(), XSD_INTEGER),
        D::I32(Some(x)) => v(x.to_string(), XSD_INTEGER),
        D::I64(Some(x)) => v(x.to_string(), XSD_INTEGER),
        D::F32(Some(x)) => v(canonical_double(widen_f32(*x)), XSD_DOUBLE),
        D::F64(Some(x)) => v(canonical_double(*x), XSD_DOUBLE),
        D::Bit(Some(b)) => v(b.to_string(), XSD_BOOLEAN),
        D::String(Some(s)) => v(s.to_string(), XSD_STRING),
        D::Guid(Some(g)) => v(g.to_string(), XSD_STRING),
        D::Binary(Some(b)) => v(
            b.iter().map(|x| format!("{x:02X}")).collect(),
            XSD_HEXBINARY,
        ),
        D::Numeric(Some(n)) => v(canonical_decimal(&n.to_string()), XSD_DECIMAL),
        D::Date(Some(_)) => NaiveDate::from_sql(&d)
            .ok()
            .flatten()
            .and_then(|x| v(x.to_string(), XSD_DATE)),
        D::Time(Some(_)) => NaiveTime::from_sql(&d)
            .ok()
            .flatten()
            .and_then(|x| v(x.to_string(), XSD_TIME)),
        D::DateTime(Some(_)) | D::SmallDateTime(Some(_)) | D::DateTime2(Some(_)) => {
            dt(NaiveDateTime::from_sql(&d).ok().flatten()).and_then(|x| v(x, XSD_DATETIME))
        }
        _ => None,
    }
}

/// XSD canonical integer: no sign for positives, no leading zeros.
pub fn canonical_integer(text: &str) -> String {
    let neg = text.starts_with('-');
    let digits = text.trim_start_matches(['+', '-']).trim_start_matches('0');
    match (digits.is_empty(), neg) {
        (true, _) => "0".to_owned(),
        (false, true) => format!("-{digits}"),
        (false, false) => digits.to_owned(),
    }
}

/// XSD canonical decimal: no trailing fractional zeros, no needless point.
pub fn canonical_decimal(text: &str) -> String {
    let neg = text.starts_with('-');
    let v = text.trim_start_matches(['+', '-']);
    let (int, frac) = v.split_once('.').unwrap_or((v, ""));
    let int = int.trim_start_matches('0');
    let int = if int.is_empty() { "0" } else { int };
    let frac = frac.trim_end_matches('0');
    let body = if frac.is_empty() {
        int.to_owned()
    } else {
        format!("{int}.{frac}")
    };
    if neg && body != "0" {
        format!("-{body}")
    } else {
        body
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cols(dialect: Dialect, names: &[&str]) -> Columns {
        Columns {
            names: names.iter().map(|s| s.to_string()).collect(),
            dialect,
        }
    }

    #[test]
    fn identifiers() {
        let pg = cols(Dialect::Postgres, &["id", "Name"]);
        assert_eq!(pg.position("ID").unwrap(), 0); // folded to id
        assert!(pg.position("\"ID\"").is_err());
        assert_eq!(pg.position("\"Name\"").unwrap(), 1);
        assert_eq!(pg.position("Name").unwrap(), 1);
        let my = cols(Dialect::MySql, &["ID", "Name"]);
        assert_eq!(my.position("name").unwrap(), 1);
        assert_eq!(query_for("\"Student\"", true), "SELECT * FROM \"Student\"");
    }

    #[test]
    fn dsn() {
        assert_eq!(
            host_port_db("jdbc:postgresql://localhost:15432/rml", 5432).unwrap(),
            ("localhost".into(), 15432, "rml".into())
        );
        assert_eq!(
            Dialect::of("jdbc:sqlserver://h:1433;databaseName=x").unwrap(),
            Dialect::SqlServer
        );
    }

    /// Runs against a real server, e.g. `docker run -d --rm -p
    /// 127.0.0.1:55432:5432 -e POSTGRES_HOST_AUTH_METHOD=trust
    /// postgres:16-alpine`, then `RMLRS_TEST_POSTGRES=jdbc:postgresql://
    /// 127.0.0.1:55432/postgres cargo test -p rmlrs-source --features
    /// postgres -- --ignored`.
    #[cfg(feature = "postgres")]
    #[test]
    #[ignore = "needs a PostgreSQL server (RMLRS_TEST_POSTGRES)"]
    fn postgres_cursor_batches() {
        let db = DatabaseSource {
            dsn: std::env::var("RMLRS_TEST_POSTGRES").expect("RMLRS_TEST_POSTGRES"),
            driver: None,
            username: Some("postgres".into()),
            password: None,
        };
        let rows = |query: &str, fetch: usize| -> Vec<Vec<Option<String>>> {
            postgres_cursor(&db, query, fetch)
                .unwrap()
                .map(|it| match it.unwrap() {
                    Iteration::Sql(row) => (row.values.into_iter())
                        .map(|v| v.map(|v| format!("{}^^{}", v.lexical, v.datatype)))
                        .collect(),
                    other => panic!("not a row: {other:?}"),
                })
                .collect()
        };
        let int = |i: i32| Some(format!("{i}^^{XSD_INTEGER}"));
        let string = |s: &str| Some(format!("{s}^^{XSD_STRING}"));
        let five = "SELECT * FROM (VALUES (1, 'a'), (2, NULL), (3, 'c'), (4, 'd'), (5, 'e')) \
                    AS t(id, name) ORDER BY id";
        let expected = vec![
            vec![int(1), string("a")],
            vec![int(2), None],
            vec![int(3), string("c")],
            vec![int(4), string("d")],
            vec![int(5), string("e")],
        ];
        // More rows than one fetch, a last fetch that is full, one row per
        // fetch, and everything in one fetch: always the same rows.
        for fetch in [2, 5, 1, 1000] {
            assert_eq!(rows(five, fetch), expected, "fetch {fetch}");
        }
        // A trailing semicolon or `--` comment does not break the cursor.
        assert_eq!(rows(&format!("{five};"), 2), expected);
        assert_eq!(rows(&format!("{five} -- five rows"), 2), expected);
        assert!(rows(&format!("{five} LIMIT 0"), 2).is_empty());
        // A larger result: 2,500 rows in three fetches of 1,000.
        let many = rows("SELECT g FROM generate_series(1, 2500) AS g", PG_FETCH_ROWS);
        assert_eq!(many.len(), 2500);
        assert_eq!(many[2499], vec![int(2500)]);
        // An error in the query is reported before any row.
        assert!(postgres_cursor(&db, "SELECT nope FROM pg_class", 2).is_err());
    }

    /// Runs against a real server, e.g. `docker run -d --rm -p
    /// 127.0.0.1:53306:3306 -e MYSQL_ROOT_PASSWORD=root mysql:8.0`, then
    /// `RMLRS_TEST_MYSQL=jdbc:mysql://127.0.0.1:53306/mysql cargo test -p
    /// rmlrs-source --features mysql -- --ignored` (user and password root).
    #[cfg(feature = "mysql")]
    #[test]
    #[ignore = "needs a MySQL server (RMLRS_TEST_MYSQL)"]
    fn mysql_rows_one_at_a_time() {
        let db = DatabaseSource {
            dsn: std::env::var("RMLRS_TEST_MYSQL").expect("RMLRS_TEST_MYSQL"),
            driver: None,
            username: Some("root".into()),
            password: Some("root".into()),
        };
        let rows = |query: &str| -> Vec<Vec<Option<String>>> {
            mysql_rows(&db, query)
                .unwrap()
                .map(|it| match it.unwrap() {
                    Iteration::Sql(row) => (row.values.into_iter())
                        .map(|v| v.map(|v| format!("{}^^{}", v.lexical, v.datatype)))
                        .collect(),
                    other => panic!("not a row: {other:?}"),
                })
                .collect()
        };
        let int = |i: i32| Some(format!("{i}^^{XSD_INTEGER}"));
        let string = |s: &str| Some(format!("{s}^^{XSD_STRING}"));
        let three = "SELECT 1 AS id, 'a' AS name UNION ALL SELECT 2, NULL \
                     UNION ALL SELECT 3, 'c'";
        assert_eq!(
            rows(three),
            vec![
                vec![int(1), string("a")],
                vec![int(2), None],
                vec![int(3), string("c")],
            ]
        );
        assert!(rows(&format!("{three} LIMIT 0")).is_empty());
        // A larger result than the socket buffers hold.
        let counting = "WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n \
                        WHERE i < 50000) SELECT";
        let many = rows(&format!(
            "{counting} /*+ SET_VAR(cte_max_recursion_depth = 50000) */ \
             i, REPEAT('x', 100) AS pad FROM n"
        ));
        assert_eq!(many.len(), 50000);
        assert_eq!(many[49999][0], int(50000));
        // An error the server reports while sending rows (here the default
        // recursion limit of 1,000) ends the rows with that error.
        let cut: Vec<_> = mysql_rows(&db, &format!("{counting} i FROM n"))
            .unwrap()
            .collect();
        assert!(cut.last().is_some_and(|r| r.is_err()));
        assert!(cut[..cut.len() - 1].iter().all(|r| r.is_ok()));
        // Stopping early leaves nothing behind: the rest is read and the
        // connection closed when the rows are dropped.
        let mut first = mysql_rows(&db, "SELECT 1 UNION ALL SELECT 2").unwrap();
        assert!(first.next().is_some());
        drop(first);
        // An error in the query is reported before any row.
        assert!(mysql_rows(&db, "SELECT nope FROM mysql.user").is_err());
    }

    #[cfg(feature = "mssql")]
    #[test]
    fn real_values_keep_their_decimal_form() {
        assert_eq!(canonical_double(widen_f32(70.22)), "7.022E1");
        assert_eq!(canonical_double(widen_f32(0.1)), "1.0E-1");
    }

    #[test]
    fn canonical_numbers() {
        assert_eq!(canonical_decimal("030.500"), "30.5");
        assert_eq!(canonical_decimal("-0.0"), "0");
        assert_eq!(canonical_integer("+0042"), "42");
    }
}
