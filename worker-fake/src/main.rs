use axum::{
    Json, Router, extract::{rejection::{JsonRejection}}, http::{StatusCode}, response::IntoResponse, routing::post,
};

use tokio::net::TcpListener;
use serde::{
    Deserialize,
    Serialize
};

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

#[derive(Serialize)]
struct ErrorBody {
    error: String
}

struct ApiError {
    status: StatusCode,
    message: String
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (self.status, Json(ErrorBody {error: self.message})).into_response()
    }
}

impl ApiError {
    fn unprocessable(message: impl Into<String>) -> Self {
        Self { status: StatusCode::UNPROCESSABLE_ENTITY, message: message.into() }
    }
}

fn validate(req: &DecideRequest) -> Result<(), ApiError> {
    if !(1..=8192).contains(&req.input.len()){
        return Err(ApiError:: unprocessable(format!("input must be 1–8192 bytes, got {}", req.input.len())));
    }
    Ok(())
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

async fn decide_handler(payload: Result<Json<DecideRequest>, JsonRejection>) -> Result<String, ApiError> {
    match payload {
        Ok(Json(req)) => {
            validate(&req)?;
            Ok(format!("{:?}", req))
        }
        Err(rejection) => {
            Err(ApiError { status: rejection.status(), message: rejection.body_text() })
        }       
    }  
}