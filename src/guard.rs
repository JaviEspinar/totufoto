//! Request checks for a server without login.
//!
//! Anyone who can reach the server can use it; these checks keep web pages on other sites
//! from using it through the visitor's browser:
//!
//! - **DNS rebinding**: a page on `evil.example` makes its name resolve to this server, so
//!   the browser treats it as the same site and lets it read and change everything. Such a
//!   request always carries the page's domain name in its `Host` header, so only an IP
//!   address, `localhost`, this computer's own names and names given with `--allow-host`
//!   are accepted.
//! - **Cross-site requests** (a form or `fetch` on another site that posts here): browsers
//!   say where a request comes from in `Sec-Fetch-Site` (or at least `Origin`), so
//!   requests that change something are refused unless they come from the gallery itself.
//!   Clients that are not browsers (curl, scripts) send neither header and are allowed.

use std::net::IpAddr;
use std::sync::Arc;

use axum::extract::{Request, State};
use axum::http::{Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

/// Host names (besides IP addresses and `localhost`) the server answers to.
#[derive(Clone, Debug, Default)]
pub struct HostNames(Vec<String>);

impl HostNames {
    /// This computer's names plus `extra` (from `--allow-host`).
    pub fn with_own_names(extra: &[String]) -> Self {
        let mut names: Vec<String> = extra.iter().map(|n| normalize(n)).collect();
        if let Some(name) = machine_name() {
            let short = name.split('.').next().unwrap_or(&name).to_string();
            names.push(format!("{short}.local"));
            names.push(short);
            names.push(name);
        }
        names.retain(|n| !n.is_empty());
        names.sort();
        names.dedup();
        Self(names)
    }

    pub fn names(&self) -> &[String] {
        &self.0
    }

    fn allows(&self, host_header: &str) -> bool {
        let host = host_part(host_header);
        if host.parse::<IpAddr>().is_ok() {
            return true;
        }
        let host = normalize(host);
        host == "localhost" || self.0.contains(&host)
    }
}

/// The host of a `Host` header without the port; IPv6 addresses without their brackets.
fn host_part(header: &str) -> &str {
    if let Some(rest) = header.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    match header.rsplit_once(':') {
        Some((host, port)) if port.chars().all(|c| c.is_ascii_digit()) => host,
        _ => header,
    }
}

fn normalize(name: &str) -> String {
    name.trim().trim_end_matches('.').to_ascii_lowercase()
}

/// The computer's name, as `hostname` prints it (the command exists on Linux, macOS and
/// Windows).
fn machine_name() -> Option<String> {
    let out = std::process::Command::new("hostname").output().ok()?;
    let name = normalize(String::from_utf8_lossy(&out.stdout).as_ref());
    (!name.is_empty() && out.status.success()).then_some(name)
}

/// Whether a request that changes something comes from the gallery's own pages.
fn same_site(req: &Request) -> bool {
    let headers = req.headers();
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        // "none": typed in the address bar or opened from a bookmark.
        return matches!(site, "same-origin" | "none");
    }
    match headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        // No Origin either: not a browser (or a very old one).
        None => true,
        Some(origin) => {
            let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
            let origin_host = origin.split_once("://").map_or(origin, |(_, rest)| rest);
            origin_host.eq_ignore_ascii_case(host)
        }
    }
}

pub async fn check(State(names): State<Arc<HostNames>>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok());
    if !host.is_some_and(|h| names.allows(h)) {
        let shown = host.unwrap_or("(none)");
        tracing::warn!("refused a request for host {shown}; if that is this computer's name, start with --allow-host");
        return (StatusCode::FORBIDDEN, format!("unexpected Host header {shown}: start Totufoto with --allow-host to use this name"))
            .into_response();
    }
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    if !safe && !same_site(&req) {
        return (StatusCode::FORBIDDEN, "requests from other sites are not allowed").into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;

    fn names(extra: &[&str]) -> HostNames {
        HostNames(extra.iter().map(|n| normalize(n)).collect())
    }

    #[test]
    fn ip_addresses_and_localhost_are_allowed() {
        let n = names(&[]);
        for h in ["127.0.0.1:7878", "192.168.0.13:7878", "10.0.0.5", "[::1]:7878", "[fe80::1]:80", "localhost:7878", "LOCALHOST"] {
            assert!(n.allows(h), "{h}");
        }
    }

    #[test]
    fn domain_names_need_to_be_known() {
        let n = names(&["Caja.local", "photos.home."]);
        assert!(n.allows("caja.local:7878"));
        assert!(n.allows("photos.home:7878"));
        assert!(n.allows("PHOTOS.HOME."));
        for h in ["evil.example:7878", "caja.local.evil.example", "localhost.evil.example", "1.2.3.4.evil.example", ""] {
            assert!(!n.allows(h), "{h}");
        }
    }

    fn post(headers: &[(&str, &str)]) -> Request {
        let mut b = Request::builder().method("POST").uri("/api/scan");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::empty()).unwrap()
    }

    #[test]
    fn cross_site_requests_are_refused() {
        assert!(same_site(&post(&[("host", "192.168.0.13:7878"), ("sec-fetch-site", "same-origin")])));
        assert!(same_site(&post(&[("host", "192.168.0.13:7878"), ("sec-fetch-site", "none")])));
        assert!(!same_site(&post(&[("host", "192.168.0.13:7878"), ("sec-fetch-site", "cross-site")])));
        assert!(!same_site(&post(&[("host", "192.168.0.13:7878"), ("sec-fetch-site", "same-site")])));
        // Browsers without Sec-Fetch-Site: Origin must be this server.
        assert!(same_site(&post(&[("host", "192.168.0.13:7878"), ("origin", "http://192.168.0.13:7878")])));
        assert!(!same_site(&post(&[("host", "192.168.0.13:7878"), ("origin", "http://evil.example")])));
        assert!(!same_site(&post(&[("host", "192.168.0.13:7878"), ("origin", "null")])));
        // Not a browser.
        assert!(same_site(&post(&[("host", "192.168.0.13:7878")])));
    }
}
