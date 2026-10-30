use axum::{
    Json, Router, routing::post,
};

use tokio::net::TcpListener;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
struct DecideRequest {
    input: String,
    question: Question
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Question {
    Choice {options: Vec<String>},
    YesNo ,
    Score {min: i8, max: i8, labels: Option<Vec<String>>}
}

#[tokio::main]
async fn main() {
    // define router with POST/decide route
    let app = Router::new().route("/decide", post(decide_handler));

    // Bind TCP listener to 127.0.0.1:9001
    let listener = TcpListener::bind("127.0.0.1:9001").await.expect("Port already in use: try another one");

    // serve app over listener
    axum::serve(listener, app).await.unwrap();
}

async fn decide_handler(Json(req):Json<DecideRequest>) -> String {
    format!("{:?}", req)
}