use tokio::net::TcpListener;

use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode, header::CONTENT_TYPE},
    response::{IntoResponse, Response},
    routing::post,
};

use serde::Serialize;

use std::time::Duration;

// Provisional upstream timeout value: 4 * worker's worst case latency with headroom in ms
const DEFAULT_UPSTREAM_TIMEOUT_MS: u64 = 700;
const CONNECT_TIMEOUT_MS: u64 = 200;

/// The JSON body of every error response
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Clone)]
struct AppState {
    client: reqwest::Client,
    worker_url: String,
}

#[tokio::main]
async fn main() {
    let worker_url =
        std::env::var("WORKER_URL").unwrap_or_else(|_| "http://127.0.0.1:9001".to_string());

    let timeout_ms: u64 = std::env::var("UPSTREAM_TIMEOUT_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_UPSTREAM_TIMEOUT_MS);

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_millis(CONNECT_TIMEOUT_MS))
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .expect("failed to build HTTP client");

    let state = AppState {
        client: client,
        worker_url,
    };

    println!(
        "gateway: worker {}, timeout {} ms",
        state.worker_url, timeout_ms
    );

    let app = Router::new()
        .route("/decide", post(forward_handler))
        .with_state(state);

    let listener = TcpListener::bind("127.0.0.1:8080").await.expect("...");
    axum::serve(listener, app).await.unwrap();
}

async fn forward_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let mut request = state
        .client
        .post(format!("{}/decide", state.worker_url))
        .body(body);
    if let Some(content_type) = headers.get(CONTENT_TYPE) {
        request = request.header(CONTENT_TYPE, content_type.clone());
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => return upstream_error(err),
    };
    let status = response.status();
    let content_type = response.headers().get(CONTENT_TYPE).cloned();
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(err) => return upstream_error(err),
    };

    let mut reply = (status, body).into_response();
    if let Some(content_type) = content_type {
        reply.headers_mut().insert(CONTENT_TYPE, content_type);
    }
    reply
}

fn upstream_error(err: reqwest::Error) -> Response {
    let (status, message) = if err.is_timeout() {
        (
            StatusCode::GATEWAY_TIMEOUT,
            format!("worker did not respond in time: {}", err),
        )
    } else {
        (
            StatusCode::BAD_GATEWAY,
            format!("worker unreachable or sent an invalid response: {}", err),
        )
    };
    (status, Json(ErrorBody { error: message })).into_response()
}
