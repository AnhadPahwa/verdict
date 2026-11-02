use axum::{
    Json, Router, extract::rejection::JsonRejection, http::StatusCode, response::IntoResponse,
    routing::post,
};

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use std::collections::{HashMap, HashSet};

#[derive(Deserialize, Debug)]
struct DecideRequest {
    input: String,
    question: Question,
}

#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Question {
    Choice {
        options: Vec<String>,
    },
    YesNo,
    Score {
        min: i8,
        max: i8,
        labels: Option<Vec<String>>,
    },
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

struct ApiError {
    status: StatusCode,
    message: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

impl ApiError {
    fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: message.into(),
        }
    }
}

fn validate(req: &DecideRequest) -> Result<(), ApiError> {
    if !(1..=8192).contains(&req.input.len()) {
        return Err(ApiError::unprocessable(format!(
            "input must be 1–8192 bytes, got {}",
            req.input.len()
        )));
    }
    match &req.question {
        Question::Choice { options } => {
            if !(2..=32).contains(&options.len()) {
                return Err(ApiError::unprocessable(format!(
                    "choice needs 2-32 options, got {}",
                    options.len()
                )));
            }
            check_unique_non_empty(options, "option")?;
        }
        Question::YesNo => {}
        Question::Score { min, max, labels } => {
            if max <= min {
                return Err(ApiError::unprocessable(format!(
                    "min must be less than max, got min {}, max {}",
                    min, max
                )));
            }
            let width = i16::from(*max) - i16::from(*min);
            if width > 10 {
                return Err(ApiError::unprocessable(format!(
                    "score scale must have at most 11 points, got {}",
                    width + 1
                )));
            }
            if let Some(labels) = labels {
                let points = width + 1;
                if points as usize != labels.len() {
                    return Err(ApiError::unprocessable(format!(
                        "score scale and number of labels do not match, got {} points, {} labels",
                        points,
                        labels.len()
                    )));
                }
                check_unique_non_empty(labels, "label")?;
            }
        }
    }
    Ok(())
}

fn check_unique_non_empty(items: &[String], what: &str) -> Result<(), ApiError> {
    let mut seen: HashSet<&String> = HashSet::new();
    for (i, item) in items.iter().enumerate() {
        if item.is_empty() {
            return Err(ApiError::unprocessable(format!("{} {} is empty", what, i)));
        }
        if !seen.insert(item) {
            return Err(ApiError::unprocessable(format!(
                "duplicate {} \"{}\"",
                what, item
            )));
        }
    }
    Ok(())
}

#[derive(Serialize)]
struct DecideResponse {
    #[serde(rename = "type")]
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<f64>,
    probabilities: HashMap<String, f64>,
    answer: String,
    model: String,
    latency_ms: f64,
}

fn options_in_order(question: &Question) -> Vec<String> {
    match question {
        Question::Choice { options } => options.clone(),
        Question::YesNo => {
            vec!["yes".to_string(), "no".to_string()]
        }
        Question::Score { min, max, .. } => (*min..=*max).map(|v| v.to_string()).collect(),
    }
}

#[tokio::main]
async fn main() {
    // define router with POST/decide route
    let app = Router::new().route("/decide", post(decide_handler));

    // Bind TCP listener to 127.0.0.1:9001
    let listener = TcpListener::bind("127.0.0.1:9001")
        .await
        .expect("Port already in use: try another one");

    // serve app over listener
    axum::serve(listener, app).await.unwrap();
}

async fn decide_handler(
    payload: Result<Json<DecideRequest>, JsonRejection>,
) -> Result<Json<DecideResponse>, ApiError> {
    match payload {
        Ok(Json(req)) => {
            validate(&req)?;
            let options = options_in_order(&req.question);
            let probability = 1.0 / options.len() as f64;
            let mut probabilities: HashMap<String, f64> = HashMap::new();
            let mut answer = &options[0];
            for option in &options {
                probabilities.insert(option.clone(), probability);
                if probabilities[option] > probabilities[answer] {
                    answer = option;
                }
            }

            let expected = match &req.question {
                Question::Score { min, max, .. } => {
                    let mut sum = 0.0;
                    for value in *min..=*max {
                        let v = value as f64;
                        sum += v * probabilities[&value.to_string()];
                    }
                    Some(sum)
                }
                _ => None,
            };

            let kind = match &req.question {
                Question::Choice { .. } => "choice".to_string(),
                Question::YesNo => "yes_no".to_string(),
                Question::Score { .. } => "score".to_string(),
            };

            let response = DecideResponse {
                kind,
                expected,
                probabilities,
                answer: answer.clone(),
                model: "fake_v0".to_string(),
                latency_ms: 0.0,
            };

            Ok(Json(response))
        }
        Err(rejection) => {
            Err(ApiError {status: rejection.status(), message: rejection.body_text()})
        },
    }
}
