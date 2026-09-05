use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use signal_hook::consts::{SIGINT, SIGTERM};
use tiny_http::{Header, Response, Server, StatusCode};

use crate::repository::{self, Repository};

/// # Errors
/// Returns an error if the server cannot bind or write fails.
#[expect(clippy::missing_panics_doc, reason = "const header parses infallibly")]
pub fn serve<O: Write>(repo: &Repository, port: u16, stdout: &mut O) -> Result<(), String> {
    let snapshot = repository::serialize(repo);

    let addr = format!("127.0.0.1:{port}");
    let server = Server::http(&addr).map_err(|e| format!("cannot bind to {addr}: {e}"))?;

    let actual_port = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| "server did not bind to an IP address".to_owned())?
        .port();

    let url = format!("http://127.0.0.1:{actual_port}/repository.json\n");
    stdout
        .write_all(url.as_bytes())
        .map_err(|e| format!("write failed: {e}"))?;
    stdout.flush().map_err(|e| format!("flush failed: {e}"))?;

    let stop = Arc::new(AtomicBool::new(false));

    signal_hook::flag::register(SIGINT, Arc::clone(&stop))
        .map_err(|e| format!("cannot register SIGINT handler: {e}"))?;
    signal_hook::flag::register(SIGTERM, Arc::clone(&stop))
        .map_err(|e| format!("cannot register SIGTERM handler: {e}"))?;

    let content_type: Header = "Content-Type: application/json; charset=utf-8"
        .parse()
        .expect("valid header");

    while !stop.load(Ordering::Relaxed) {
        let request = match server.recv_timeout(std::time::Duration::from_millis(100)) {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(_) => break,
        };

        if request.url() != "/repository.json" {
            let resp = Response::empty(StatusCode(404));
            let _ = request.respond(resp);
            continue;
        }

        match request.method() {
            tiny_http::Method::Get => {
                let resp =
                    Response::from_data(snapshot.as_bytes()).with_header(content_type.clone());
                let _ = request.respond(resp);
            }
            tiny_http::Method::Head => {
                let resp = Response::empty(StatusCode(200)).with_header(content_type.clone());
                let _ = request.respond(resp);
            }
            _ => {
                let allow: Header = "Allow: GET, HEAD".parse().expect("valid header");
                let resp = Response::empty(StatusCode(405)).with_header(allow);
                let _ = request.respond(resp);
            }
        }
    }

    Ok(())
}

/// # Errors
/// Returns an error on network failure, non-200 status, or invalid repository JSON.
pub fn fetch_remote_repository(url: &str) -> Result<Repository, String> {
    let agent = ureq::config::Config::builder()
        .max_redirects(0)
        .build()
        .new_agent();

    let response = agent.get(url).call().map_err(|e| match e {
        ureq::Error::StatusCode(code) => format!("HTTP {code}"),
        other => format!("HTTP request failed: {other}"),
    })?;

    let status = response.status();
    if status != 200 {
        return Err(format!("HTTP {status}"));
    }

    let body = response
        .into_body()
        .read_to_string()
        .map_err(|e| format!("failed to read response body: {e}"))?;

    repository::parse(&body).map_err(|e| format!("invalid JSON: {e}"))
}

#[must_use]
pub fn is_http_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_http_url_recognizes_http() {
        assert!(is_http_url("http://127.0.0.1:8765/repository.json"));
    }

    #[test]
    fn is_http_url_recognizes_https() {
        assert!(is_http_url("https://example.com/repository.json"));
    }

    #[test]
    fn is_http_url_rejects_local_path() {
        assert!(!is_http_url("../other-repo"));
        assert!(!is_http_url("/absolute/path"));
        assert!(!is_http_url("relative/path"));
    }
}
