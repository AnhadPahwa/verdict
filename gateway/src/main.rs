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

    let state = AppState {
        client: reqwest::Client::new(),
        worker_url,
    };

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
        Err(err) => return bad_gateway(format!("Worker unreachable: {}", err)),
    };
    let status = response.status();
    let content_type = response.headers().get(CONTENT_TYPE).cloned();
    let body = match response.bytes().await {
        Ok(body) => body,
        Err(err) => return bad_gateway(format!("Failed to read worker response: {}", err)),
    };

    let mut reply = (status, body).into_response();
    if let Some(content_type) = content_type {
        reply.headers_mut().insert(CONTENT_TYPE, content_type);
    }
    reply
}

fn bad_gateway(message: String) -> Response {
    (StatusCode::BAD_GATEWAY, Json(ErrorBody { error: message })).into_response()
}
