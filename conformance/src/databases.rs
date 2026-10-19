// SPDX-License-Identifier: Apache-2.0
//! Databases for the SQL test cases, in Docker containers.
//!
//! Each SQL case ships a `resource.sql` fixture and a mapping whose
//! `d2rq:jdbcDSN` is the placeholder `CONNECTIONDSN`. The runner resets the
//! database, loads the fixture and substitutes the placeholder.

use std::process::Command;
use std::time::{Duration, Instant};

const PG_CONTAINER: &str = "rmlrs-conformance-postgres";
const PG_IMAGE: &str = "postgres:16-alpine";
/// The runner only ever connects to this database, which exists only in
/// its own container. A fixed port once collided with a port-forward to
/// another PostgreSQL server; with a dedicated database name and a free
/// port picked at start-up, the runner cannot reset someone else's data.
const PG_DB: &str = "rmlrs_conformance";

const MY_CONTAINER: &str = "rmlrs-conformance-mysql";
const MY_IMAGE: &str = "mysql:8.4";
const MS_CONTAINER: &str = "rmlrs-conformance-mssql";
const MS_IMAGE: &str = "mcr.microsoft.com/mssql/server:2022-latest";
/// The password the SQL Server cases' mappings carry.
const MS_PASSWORD: &str = "YourSTRONG!Passw0rd;";

type MsClient = tiberius::Client<tokio_util::compat::Compat<tokio::net::TcpStream>>;

/// A TCP port that is free on 127.0.0.1 right now.
fn free_port() -> Result<u16, String> {
    let l = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| e.to_string())?;
    Ok(l.local_addr().map_err(|e| e.to_string())?.port())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Db {
    Postgres,
    MySql,
    SqlServer,
}

/// Which database a case is written for. The title names it ("PostgreSQL:
/// ..."); two cases name one database and give another's driver, and the
/// title is the one that matches what they test.
pub fn db_of(title: &str, mapping: &str) -> Option<Db> {
    let t = title.trim_start_matches('"');
    if t.starts_with("PostgreSQL") {
        return Some(Db::Postgres);
    }
    if t.starts_with("MySQL") {
        return Some(Db::MySql);
    }
    if t.starts_with("SQLServer") || t.starts_with("SQL Server") {
        return Some(Db::SqlServer);
    }
    if mapping.contains("org.postgresql.Driver") {
        Some(Db::Postgres)
    } else if mapping.contains("com.mysql") {
        Some(Db::MySql)
    } else if mapping.contains("sqlserver") {
        Some(Db::SqlServer)
    } else {
        None
    }
}

/// The databases this run has.
#[derive(Default)]
pub struct Databases {
    postgres: Option<postgres::Client>,
    pg_port: u16,
    mysql: Option<mysql::Conn>,
    my_port: u16,
    mssql: Option<(tokio::runtime::Runtime, MsClient)>,
    ms_port: u16,
}

impl std::fmt::Debug for Databases {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Databases")
            .field("postgres", &self.postgres.is_some())
            .finish()
    }
}

impl Databases {
    pub fn start() -> Result<Databases, String> {
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", PG_CONTAINER])
            .output();
        let port = free_port()?;
        let out = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                PG_CONTAINER,
                "-p",
                &format!("127.0.0.1:{port}:5432"),
            ])
            .args([
                "-e",
                "POSTGRES_HOST_AUTH_METHOD=trust",
                "-e",
                &format!("POSTGRES_DB={PG_DB}"),
                PG_IMAGE,
            ])
            .output()
            .map_err(|e| format!("docker: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "docker run: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            match postgres::Client::connect(
                &format!("host=127.0.0.1 port={port} user=postgres dbname={PG_DB}"),
                postgres::NoTls,
            ) {
                Ok(mut c) => {
                    // Some PostgreSQL cases carry the MySQL or SQL Server user
                    // of another case; with trust auth any role that exists works.
                    c.batch_execute(
                        "CREATE ROLE root LOGIN SUPERUSER; CREATE ROLE sa LOGIN SUPERUSER;",
                    )
                    .map_err(|e| e.to_string())?;
                    let mut dbs = Databases {
                        postgres: Some(c),
                        pg_port: port,
                        mysql: None,
                        my_port: 0,
                        mssql: None,
                        ms_port: 0,
                    };
                    if let Err(e) = dbs.start_mysql().and_then(|()| dbs.start_mssql()) {
                        dbs.stop();
                        return Err(e);
                    }
                    return Ok(dbs);
                }
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(500))
                }
                Err(e) => return Err(format!("PostgreSQL did not start: {e:?}")),
            }
        }
    }

    /// MySQL in its own container on another free port. The runner's own
    /// connection goes to the `rmlrs_conformance` database, which only this
    /// container has; fixtures then create the `test` database they use.
    fn start_mysql(&mut self) -> Result<(), String> {
        use mysql::prelude::Queryable;
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", MY_CONTAINER])
            .output();
        let port = free_port()?;
        let out = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--name",
                MY_CONTAINER,
                "-p",
                &format!("127.0.0.1:{port}:3306"),
            ])
            .args([
                "-e",
                "MYSQL_ALLOW_EMPTY_PASSWORD=yes",
                "-e",
                &format!("MYSQL_DATABASE={PG_DB}"),
                MY_IMAGE,
            ])
            .output()
            .map_err(|e| format!("docker: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "docker run mysql: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let opts = mysql::OptsBuilder::new()
            .ip_or_hostname(Some("127.0.0.1"))
            .tcp_port(port)
            .user(Some("root"))
            .db_name(Some(PG_DB));
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            match mysql::Conn::new(opts.clone()) {
                Ok(mut c) => {
                    // Users of other cases' mappings, with no password.
                    for u in ["postgres", "sa"] {
                        c.query_drop(format!("CREATE USER IF NOT EXISTS '{u}'@'%'"))
                            .and_then(|_| c.query_drop(format!("GRANT ALL ON *.* TO '{u}'@'%'")))
                            .map_err(|e| e.to_string())?;
                    }
                    self.mysql = Some(c);
                    self.my_port = port;
                    return Ok(());
                }
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(1000))
                }
                Err(e) => return Err(format!("MySQL did not start: {e}")),
            }
        }
    }

    /// SQL Server (amd64 image; emulated on Apple Silicon) on a free port.
    /// Before using it, the runner checks that the server's name is this
    /// container's host name, so it never resets another SQL Server.
    fn start_mssql(&mut self) -> Result<(), String> {
        let _ = Command::new("docker")
            .args(["rm", "-f", "-v", MS_CONTAINER])
            .output();
        let port = free_port()?;
        let out = Command::new("docker")
            .args([
                "run",
                "-d",
                "--rm",
                "--platform",
                "linux/amd64",
                "--name",
                MS_CONTAINER,
            ])
            .args([
                "-p",
                &format!("127.0.0.1:{port}:1433"),
                "-e",
                "ACCEPT_EULA=Y",
            ])
            .args(["-e", &format!("MSSQL_SA_PASSWORD={MS_PASSWORD}"), MS_IMAGE])
            .output()
            .map_err(|e| format!("docker: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "docker run mssql: {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }
        let container_id = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            match rt.block_on(ms_connect(port)) {
                Ok(mut c) => {
                    let name = rt
                        .block_on(async {
                            c.simple_query("SELECT @@SERVERNAME")
                                .await?
                                .into_row()
                                .await
                        })
                        .map_err(|e| e.to_string())?
                        .and_then(|r| r.get::<&str, _>(0).map(str::to_owned))
                        .unwrap_or_default();
                    if !container_id.starts_with(&name.to_lowercase()) {
                        return Err(format!(
                            "SQL Server on port {port} is not this runner's container ({name})"
                        ));
                    }
                    // Users of other cases' mappings: empty passwords need CHECK_POLICY OFF.
                    let users = "CREATE LOGIN root WITH PASSWORD = '', CHECK_POLICY = OFF; ALTER SERVER ROLE sysadmin ADD MEMBER root; \
                                 CREATE LOGIN postgres WITH PASSWORD = '', CHECK_POLICY = OFF; ALTER SERVER ROLE sysadmin ADD MEMBER postgres;";
                    rt.block_on(async { c.simple_query(users).await?.into_results().await })
                        .map_err(|e| e.to_string())?;
                    self.mssql = Some((rt, c));
                    self.ms_port = port;
                    return Ok(());
                }
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(2000))
                }
                Err(e) => return Err(format!("SQL Server did not start: {e}")),
            }
        }
    }

    /// Removes every container this runner may have started, including one
    /// left behind by a start that failed half-way.
    pub fn stop(&mut self) {
        self.postgres = None;
        self.mysql = None;
        self.mssql = None;
        for name in [PG_CONTAINER, MY_CONTAINER, MS_CONTAINER] {
            // -v: the database images declare volumes; without it every run
            // leaves anonymous volumes behind.
            let _ = Command::new("docker")
                .args(["rm", "-f", "-v", name])
                .output();
        }
    }

    /// Resets the database, loads the fixture and returns the JDBC string
    /// that replaces `CONNECTIONDSN`.
    pub fn prepare(&mut self, db: Db, fixture: &str) -> Result<String, String> {
        match (db, self.postgres.as_mut()) {
            (Db::Postgres, Some(c)) => {
                c.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
                    .map_err(|e| e.to_string())?;
                if c.batch_execute(fixture).is_err() {
                    // A fixture written for another database (`USE test;`):
                    // run its statements one by one and skip those that fail.
                    for stmt in fixture.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                        let _ = c.batch_execute(stmt);
                    }
                }
                Ok(format!(
                    "jdbc:postgresql://127.0.0.1:{}/{PG_DB}",
                    self.pg_port
                ))
            }
            (Db::MySql, _) if self.mysql.is_some() => {
                use mysql::prelude::Queryable;
                let c = self.mysql.as_mut().expect("checked");
                c.query_drop("DROP DATABASE IF EXISTS test; CREATE DATABASE test; USE test;")
                    .map_err(|e| e.to_string())?;
                if c.query_drop(fixture).is_err() {
                    c.query_drop("USE test;").map_err(|e| e.to_string())?;
                    for stmt in fixture.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                        let _ = c.query_drop(stmt);
                    }
                }
                c.query_drop(format!("USE {PG_DB};"))
                    .map_err(|e| e.to_string())?;
                Ok(format!("jdbc:mysql://127.0.0.1:{}/test", self.my_port))
            }
            (Db::SqlServer, _) if self.mssql.is_some() => {
                let (rt, c) = self.mssql.as_mut().expect("checked");
                rt.block_on(async {
                    let reset = "USE master; IF DB_ID('TestDB') IS NOT NULL BEGIN ALTER DATABASE TestDB SET SINGLE_USER WITH ROLLBACK IMMEDIATE; DROP DATABASE TestDB; END; CREATE DATABASE TestDB;";
                    c.simple_query(reset).await?.into_results().await?;
                    let loaded = match c.simple_query(fixture).await {
                        Ok(stream) => stream.into_results().await.is_ok(),
                        Err(_) => false,
                    };
                    if !loaded {
                        for stmt in fixture.split(';').map(str::trim).filter(|s| !s.is_empty()) {
                            if let Ok(s) = c.simple_query(stmt).await {
                                let _ = s.into_results().await;
                            }
                        }
                    }
                    c.simple_query("USE master;").await?.into_results().await?;
                    Ok::<_, tiberius::error::Error>(())
                })
                .map_err(|e| e.to_string())?;
                Ok(format!(
                    "jdbc:sqlserver://127.0.0.1:{};databaseName=TestDB",
                    self.ms_port
                ))
            }
            (other, _) => Err(format!("not supported: {other:?} database (not started)")),
        }
    }
}

async fn ms_connect(port: u16) -> Result<MsClient, String> {
    use tokio_util::compat::TokioAsyncWriteCompatExt;
    let mut config = tiberius::Config::new();
    config.host("127.0.0.1");
    config.port(port);
    config.authentication(tiberius::AuthMethod::sql_server("sa", MS_PASSWORD));
    config.trust_cert();
    let tcp = tokio::net::TcpStream::connect(config.get_addr())
        .await
        .map_err(|e| e.to_string())?;
    tiberius::Client::connect(config, tcp.compat_write())
        .await
        .map_err(|e| e.to_string())
}
