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
use bytes::Bytes;
use futures_util::{StreamExt, TryStreamExt};

use crate::budget::Budget;
use crate::record::{Record, ReqRecord};
use crate::store;
use crate::throughput;

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
    let local_addr = listener.local_addr()?;

    // --- Readiness seam (ADR-0002 fail-open). ---
    // The bind above is the one thing that can fail at startup; past it the Proxy
    // is bound and about to serve. Emit a single machine-readable readiness line
    // on STDOUT, flushed, so `cca` can wait on it deterministically before it
    // commits capture (exports ANTHROPIC_BASE_URL). Human messages stay on stderr;
    // stdout carries only this line, which is `cca`'s to consume — it never
    // reaches the `claude` client. If the bind had failed, `run` returns early via
    // `?` above and this line is never printed.
    println!("{}", ready_line(&local_addr));
    use std::io::Write as _;
    let _ = std::io::stdout().flush();

    eprintln!(
        "claude-dash proxy: session {id} listening on http://{local_addr} -> {UPSTREAM_BASE}"
    );
    eprintln!("set ANTHROPIC_BASE_URL=http://{local_addr}");

    axum::serve(listener, app)
        .await
        .context("serving proxy")?;
    Ok(())
}

/// The single machine-readable readiness line `cca` waits on (ADR-0002).
///
/// Printed to stdout once the listener is bound and the Proxy is about to serve,
/// so `cca` can confirm the Proxy is reachable before committing capture. Carries
/// the bound address (the OS may have resolved port `0` to an ephemeral one) so
/// `cca` could cross-check it if it wished.
fn ready_line(addr: &SocketAddr) -> String {
    format!("READY http://{addr}")
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

    // Forward request headers untouched, except the Host (must match upstream)
    // and Accept-Encoding. We force `identity` so upstream returns an unencoded
    // body: the Throughput tee parses `usage` out of the response bytes directly,
    // and a compressed (gzip/zstd/br) body would carry no literal `usage` string,
    // silently yielding zero Throughput. The relayed body stays untouched — it is
    // simply identity-encoded — and the client decodes identity transparently.
    let mut fwd_headers = parts.headers.clone();
    fwd_headers.remove(axum::http::header::HOST);
    fwd_headers.insert(
        axum::http::header::ACCEPT_ENCODING,
        HeaderValue::from_static("identity"),
    );
    upstream_req = upstream_req.headers(fwd_headers);

    let upstream_resp = upstream_req.send().await.map_err(ProxyError::Upstream)?;

    let status = upstream_resp.status();
    let headers = upstream_resp.headers().clone();

    // --- Capture Budget from the response HEAD, before touching the body. ---
    // For a /v1/messages response, read the unified rate-limit headers. Header
    // access does not consume the body stream. We hold the Budget reading and
    // append the combined `req` record only once the teed body completes (so the
    // record carries the per-Session Throughput too), but the *Budget* itself is
    // already known here.
    let budget = is_messages.then(|| budget_from_headers(&headers)).flatten();

    // --- Relay the response body, teed, streamed, and untouched. ---
    let body_stream = upstream_resp.bytes_stream();

    let relay_body = if is_messages {
        // Tee the body: every chunk is forwarded to the client *immediately* as
        // it arrives, and a copy is sent down a side channel to be parsed for
        // `usage`. The tee never awaits the parser before forwarding, so client
        // (token-by-token) streaming is never delayed or buffered. `Bytes` is
        // cheap to clone (refcounted), so teeing adds no copy of the payload.
        let (tee_tx, tee_rx) = tokio::sync::mpsc::unbounded_channel::<Bytes>();

        // The side task drains the teed copy off the relay hot path, reassembles
        // the body, parses Throughput at end-of-stream, and appends the combined
        // `req` record — keeping the blocking file write on the blocking pool.
        let session_file = state.session_file.clone();
        tokio::spawn(capture_throughput(tee_rx, budget, session_file));

        let teed = body_stream.map(move |chunk| {
            if let Ok(bytes) = &chunk {
                // Best-effort tee: if the side task has gone away the send fails
                // and we simply stop capturing — relay continues unaffected.
                let _ = tee_tx.send(bytes.clone());
            }
            chunk
        });
        Body::from_stream(teed)
    } else {
        Body::from_stream(body_stream)
    };

    let mut response = Response::builder()
        .status(reqwest_status_to_axum(status))
        .body(relay_body)
        .map_err(|e| ProxyError::Build(e.to_string()))?;

    copy_response_headers(&headers, response.headers_mut());
    Ok(response)
}

/// Drain the teed copy of a `/v1/messages` response body, parse the
/// per-**Session** **Throughput** out of the (reassembled) SSE stream, and append
/// the combined `req` record carrying both **Budget** and **Throughput**.
///
/// Runs as a side task off the relay hot path: client chunks are forwarded the
/// instant they arrive, while this accumulates the teed copy and only writes the
/// record once the stream ends (when `usage.output_tokens` is final). The blocking
/// file append is kept on the blocking pool, and a capture problem never affects
/// the client request.
async fn capture_throughput(
    mut tee_rx: tokio::sync::mpsc::UnboundedReceiver<Bytes>,
    budget: Option<Budget>,
    session_file: PathBuf,
) {
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = tee_rx.recv().await {
        body.extend_from_slice(&chunk);
    }

    let throughput = throughput::parse_usage(&body);

    // Budget is the record's spine (it embeds a Budget); with no Budget reading
    // there's no `req` record to write this slice. Throughput still rode the tee
    // for free and attaches as soon as a Budget reading is present.
    let Some(budget) = budget else {
        return;
    };

    let ts = chrono::Utc::now().timestamp_millis();
    let record = Record::Req(ReqRecord::from_budget(&budget, ts, throughput));

    tokio::task::spawn_blocking(move || {
        if let Err(e) = store::append_record(&session_file, &record) {
            eprintln!("claude-dash proxy: failed to append req record: {e:#}");
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_line_is_the_single_token_cca_waits_on() {
        let addr: SocketAddr = "127.0.0.1:8787".parse().unwrap();
        let line = ready_line(&addr);
        // First token is the unambiguous READY sentinel cca greps for.
        assert!(line.starts_with("READY "), "got {line:?}");
        // Carries the bound address so cca can cross-check the resolved port.
        assert!(line.contains("127.0.0.1:8787"), "got {line:?}");
        // Single line — must not corrupt cca's line-oriented read.
        assert!(!line.contains('\n'), "got {line:?}");
    }
}
