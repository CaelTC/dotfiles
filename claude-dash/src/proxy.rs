//! The **Proxy** — a local streaming reverse-proxy.
//!
//! The client points `ANTHROPIC_BASE_URL` at it; it forwards every request to
//! `https://api.anthropic.com`, relays the response **body untouched and
//! streamed** (so the client `claude` TUI stays live — the whole body is never
//! buffered), and reads the `anthropic-ratelimit-unified-*` headers off each
//! `/v1/messages` response to capture **Budget**.
//!
//! Only the inference base is redirected here; token refresh and non-inference
//! hosts are not this proxy's concern (the client only points its inference
//! base at us). The **Proxy** self-generates a **Session** id if none is passed.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::{Context, Result};
use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::Response;
use axum::routing::any;
use axum::Router;
use futures_util::TryStreamExt;

use crate::budget::Budget;
use crate::record::{Record, ReqRecord};
use crate::store;

/// The inference base every request is forwarded to. Only inference is
/// redirected through the **Proxy**.
const UPSTREAM_BASE: &str = "https://api.anthropic.com";

/// Shared state for the **Proxy**: the upstream HTTP client and the
/// **Session**'s store file.
#[derive(Clone)]
struct ProxyState {
    client: reqwest::Client,
    /// `~/.cca/sessions/<id>.jsonl` for this **Session**.
    session_file: PathBuf,
}

/// Run the **Proxy** on `addr`, capturing into the **Session** `id` (one is
/// minted if `None`). Forwards everything to [`UPSTREAM_BASE`].
pub async fn run(addr: SocketAddr, id: Option<String>) -> Result<()> {
    let id = id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let dir = store::sessions_dir()?;
    // Create the store dir once at startup so the per-request append doesn't
    // repeat the syscall on the relay path.
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating store dir {}", dir.display()))?;
    let session_file = store::session_path(&dir, &id);

    let state = ProxyState {
        // No body buffering on the client side — reqwest streams by default and
        // we consume the response as a byte stream.
        client: reqwest::Client::builder()
            .build()
            .context("building upstream HTTP client")?,
        session_file,
    };

    let app = Router::new().fallback(any(handle)).with_state(state);

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("binding proxy listener on {addr}"))?;

    eprintln!(
        "claude-dash proxy: session {id} listening on http://{} -> {UPSTREAM_BASE}",
        listener.local_addr()?
    );
    eprintln!("set ANTHROPIC_BASE_URL=http://{}", listener.local_addr()?);

    axum::serve(listener, app)
        .await
        .context("serving proxy")?;
    Ok(())
}

/// Forward one request to the upstream inference base and relay the response.
///
/// The response **body is streamed untouched** — never collected into memory —
/// so the client `claude` TUI stays live with no added latency. **Budget**
/// headers are read off the response head, which is available *before and
/// independent of* the body, so header capture never drains the body.
async fn handle(State(state): State<ProxyState>, req: Request) -> Result<Response, ProxyError> {
    let (parts, body) = req.into_parts();

    // Rebuild the upstream URL: same path + query against the inference base.
    let path_and_query = parts
        .uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(parts.uri.path());
    let is_messages = parts.uri.path() == "/v1/messages";
    let upstream_url = format!("{UPSTREAM_BASE}{path_and_query}");

    // Stream the *request* body upstream rather than buffering it.
    let req_stream = body.into_data_stream().map_err(std::io::Error::other);
    let reqwest_body = reqwest::Body::wrap_stream(req_stream);

    let mut upstream_req = state
        .client
        .request(parts.method.clone(), &upstream_url)
        .body(reqwest_body);

    // Forward request headers untouched, except the Host (must match upstream).
    let mut fwd_headers = parts.headers.clone();
    fwd_headers.remove(axum::http::header::HOST);
    upstream_req = upstream_req.headers(fwd_headers);

    let upstream_resp = upstream_req.send().await.map_err(ProxyError::Upstream)?;

    let status = upstream_resp.status();
    let headers = upstream_resp.headers().clone();

    // --- Capture Budget from the response HEAD, before touching the body. ---
    // For a /v1/messages response, read the unified rate-limit headers and
    // append a `req` record. Header access does not consume the body stream.
    if is_messages {
        if let Some(budget) = budget_from_headers(&headers) {
            let ts = chrono::Utc::now().timestamp_millis();
            let record = Record::Req(ReqRecord::from_budget(&budget, ts));
            let session_file = state.session_file.clone();
            // Capture is fire-and-forget and must never block the relay path or
            // a runtime worker — do the blocking file append on the blocking
            // pool, and never fail the client request over a capture problem.
            tokio::task::spawn_blocking(move || {
                if let Err(e) = store::append_record(&session_file, &record) {
                    eprintln!("claude-dash proxy: failed to append req record: {e:#}");
                }
            });
        }
    }

    // --- Relay the response body, streamed and untouched. ---
    let body_stream = upstream_resp.bytes_stream();
    let relay_body = Body::from_stream(body_stream);

    let mut response = Response::builder()
        .status(reqwest_status_to_axum(status))
        .body(relay_body)
        .map_err(|e| ProxyError::Build(e.to_string()))?;

    copy_response_headers(&headers, response.headers_mut());
    Ok(response)
}

/// Read a [`Budget`] from a reqwest [`HeaderMap`] using the unified rate-limit
/// header names. Thin adapter over the pure [`Budget::from_headers`].
fn budget_from_headers(headers: &reqwest::header::HeaderMap) -> Option<Budget> {
    Budget::from_headers(|name| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string())
    })
}

/// Copy upstream response headers onto the relayed response untouched.
fn copy_response_headers(from: &reqwest::header::HeaderMap, to: &mut HeaderMap) {
    for (name, value) in from.iter() {
        if let (Ok(n), Ok(v)) = (
            axum::http::HeaderName::from_bytes(name.as_str().as_bytes()),
            HeaderValue::from_bytes(value.as_bytes()),
        ) {
            to.append(n, v);
        }
    }
}

/// Map a reqwest [`reqwest::StatusCode`] onto an axum [`StatusCode`] unchanged.
fn reqwest_status_to_axum(status: reqwest::StatusCode) -> StatusCode {
    StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY)
}

/// Errors the **Proxy** can surface to the client. The upstream/build distinction
/// only affects how we report; the client always gets a clean HTTP status.
#[derive(Debug)]
enum ProxyError {
    Upstream(reqwest::Error),
    Build(String),
}

impl axum::response::IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let msg = match self {
            ProxyError::Upstream(e) => format!("upstream error: {e}"),
            ProxyError::Build(e) => format!("proxy build error: {e}"),
        };
        eprintln!("claude-dash proxy: {msg}");
        (StatusCode::BAD_GATEWAY, msg).into_response()
    }
}
