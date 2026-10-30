use axum::{
    routing::post,
    Router,
};

use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    // define router with POST/decide route
    let app = Router::new().route("/decide", post(decide_handler));

    // Bind TCP listener to 127.0.0.1:9001
    let listener = TcpListener::bind("127.0.0.1:9001").await.expect("Port already in use: try another one");

    // serve app over listener
    axum::serve(listener, app).await.unwrap();
}

async fn decide_handler() -> &'static str {
    "ok"
}