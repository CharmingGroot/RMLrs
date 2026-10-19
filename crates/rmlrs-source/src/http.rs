// SPDX-License-Identifier: Apache-2.0
//! Remote sources over HTTP(S) (feature `http`): one GET per document, or
//! one POST per SPARQL query, with the response read as a stream.
//!
//! TLS is rustls with the ring provider and the operating system's root
//! certificates: no OpenSSL and no certificate list bundled in the binary.
//! There is no retrying or polling; a failed request is a source error.

use std::io::Read;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rmlrs_model::{Error, RemoteSource, Result, SparqlResultsFormat};
use ureq::tls::{Certificate, RootCerts, TlsConfig, TlsProvider};

/// One client for the process: it keeps connections and the root
/// certificates, which are slow to load.
fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        // Certificates the system store cannot parse are skipped, as a
        // browser would; a server signed by none of them fails to connect.
        let native = rustls_native_certs::load_native_certs();
        let roots: Vec<Certificate<'static>> = native
            .certs
            .iter()
            .map(|c| Certificate::from_der(c.as_ref()).to_owned())
            .collect();
        let tls = TlsConfig::builder()
            .provider(TlsProvider::Rustls)
            .root_certs(RootCerts::new_with_certs(&roots))
            .unversioned_rustls_crypto_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .build();
        ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(30)))
            .build()
            .into()
    })
}

fn source_error(url: &str, e: ureq::Error) -> Error {
    Error::Source(format!("{url}: {e}"))
}

/// Fetches a remote document. The response body is streamed; a status
/// other than 2xx is a source error.
pub fn get(source: &RemoteSource) -> Result<Box<dyn Read>> {
    let mut request = agent().get(&source.url);
    if let Some(accept) = &source.content_type {
        request = request.header("Accept", accept);
    }
    for (name, value) in &source.headers {
        request = request.header(name, value);
    }
    let response = request.call().map_err(|e| source_error(&source.url, e))?;
    Ok(Box::new(response.into_body().into_reader()))
}

/// Sends a query to a SPARQL endpoint (SPARQL 1.1 Protocol, query via
/// URL-encoded POST) and returns the results in `format`, streamed.
pub fn sparql_query(
    endpoint: &str,
    query: &str,
    format: SparqlResultsFormat,
) -> Result<Box<dyn Read>> {
    let response = agent()
        .post(endpoint)
        .header("Accept", format.media_type())
        .send_form([("query", query)])
        .map_err(|e| source_error(endpoint, e))?;
    Ok(Box::new(response.into_body().into_reader()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread::JoinHandle;

    /// A request as the stub server saw it.
    #[derive(Debug)]
    pub struct Seen {
        pub request_line: String,
        pub headers: Vec<(String, String)>,
        pub body: String,
    }

    /// Serves one HTTP request on a local port with `status` and `body`,
    /// and returns the URL and what the client sent.
    pub fn serve_once(status: &str, content_type: &str, body: &str) -> (String, JoinHandle<Seen>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/data", listener.local_addr().unwrap());
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                let (k, v) = line.split_once(':').unwrap();
                headers.push((k.trim().to_ascii_lowercase(), v.trim().to_owned()));
            }
            let length = headers
                .iter()
                .find(|(k, _)| k == "content-length")
                .map_or(0, |(_, v)| v.parse().unwrap());
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            (&stream).write_all(response.as_bytes()).unwrap();
            Seen {
                request_line: request_line.trim_end().to_owned(),
                headers,
                body: String::from_utf8(body).unwrap(),
            }
        });
        (url, handle)
    }

    #[test]
    fn get_sends_accept_and_headers() {
        let (url, server) = serve_once("200 OK", "application/json", r#"[{"id":1}]"#);
        let source = RemoteSource {
            url,
            content_type: Some("application/json".into()),
            headers: vec![("User-Agent".into(), "rmlrs-test".into())],
        };
        let mut text = String::new();
        get(&source).unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, r#"[{"id":1}]"#);
        let seen = server.join().unwrap();
        assert!(seen.request_line.starts_with("GET /data "), "{seen:?}");
        assert!(seen.body.is_empty());
        let header = |k: &str| seen.headers.iter().find(|(h, _)| h == k).map(|(_, v)| v);
        assert_eq!(header("accept").unwrap(), "application/json");
        assert_eq!(header("user-agent").unwrap(), "rmlrs-test");
    }

    #[test]
    fn sparql_queries_follow_the_protocol() {
        let (url, server) = serve_once("200 OK", "text/csv", "x\r\nhttp://e/a\r\n");
        let query = "SELECT ?x WHERE { ?x a <http://e/C> }";
        let mut text = String::new();
        sparql_query(&url, query, SparqlResultsFormat::Csv)
            .unwrap()
            .read_to_string(&mut text)
            .unwrap();
        assert_eq!(text, "x\r\nhttp://e/a\r\n");
        let seen = server.join().unwrap();
        assert!(seen.request_line.starts_with("POST /data "), "{seen:?}");
        let header = |k: &str| seen.headers.iter().find(|(h, _)| h == k).map(|(_, v)| v);
        assert_eq!(header("accept").unwrap(), "text/csv");
        assert_eq!(
            header("content-type").unwrap(),
            "application/x-www-form-urlencoded"
        );
        // The body is the URL-encoded query.
        let encoded = seen.body.strip_prefix("query=").unwrap();
        let decoded = decode_form(encoded);
        assert_eq!(decoded, query);
    }

    /// Decodes an `application/x-www-form-urlencoded` value.
    fn decode_form(s: &str) -> String {
        let mut out = Vec::new();
        let mut bytes = s.bytes();
        while let Some(b) = bytes.next() {
            match b {
                b'+' => out.push(b' '),
                b'%' => {
                    let hex: String = bytes.by_ref().take(2).map(char::from).collect();
                    out.push(u8::from_str_radix(&hex, 16).unwrap());
                }
                b => out.push(b),
            }
        }
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn error_status_is_a_source_error() {
        let (url, server) = serve_once("404 Not Found", "text/plain", "gone");
        let source = RemoteSource {
            url,
            content_type: None,
            headers: vec![],
        };
        assert!(matches!(get(&source), Err(Error::Source(_))));
        server.join().unwrap();
    }
}
