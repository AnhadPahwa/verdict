//! Fake inference worker for verdict.
//!
//! Implements the `/decide` contract in the README without a real model, so the
//! gateway can be built and benchmarked before the model exists. It is replaced
//! later by a worker serving the trained model behind the same contract

use std::collections::{HashMap, HashSet};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use axum::{
    Json, Router,
    extract::rejection::JsonRejection,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

// ---------------------------------------------------------------------------
// Fake model parameters: the simulated cost of one forward pass in microseconds
// ---------------------------------------------------------------------------

const BASE_MS: u64 = 20000; // Base cost of one forward pass
const COST_PER_BYTE: u64 = 10; // Cost per byte of input
const COST_PER_OPTION: u64 = 2000; // Cost per option in the question

// ---------------------------------------------------------------------------
// Contract types: the request and response shapes from the README.
// serde enforces the structural rules (field names and types); `validate`
// enforces the rest
// ---------------------------------------------------------------------------

/// A `/decide` request body.
#[derive(Deserialize, Debug)]
struct DecideRequest {
    input: String,
    question: Question,
}

/// Three question types. The JSON `"type"` field selects the variant,
/// e.g: `"yes_no"` becomes `Question::YesNo`.
#[derive(Deserialize, Debug)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Question {
    Choice {
        options: Vec<String>,
    },
    YesNo,
    Score {
        // i8 means min and max must be between -128 and 127 (see Clarifications).
        min: i8,
        max: i8,
        labels: Option<Vec<String>>,
    },
}

/// A successful `/decide` response.
#[derive(Serialize)]
struct DecideResponse {
    // `type` is a Rust keyword, so the field is `kind` and serde renames it
    #[serde(rename = "type")]
    kind: String,
    // Left out of the JSON entirely (not `null`) for choice and yes_no
    #[serde(skip_serializing_if = "Option::is_none")]
    expected: Option<f64>,
    probabilities: HashMap<String, f64>,
    answer: String,
    model: String,
    latency_ms: f64,
}

// ---------------------------------------------------------------------------
// Errors: every error response is `{"error": "..."}` served as JSON
// ---------------------------------------------------------------------------

/// The JSON body of every error response
#[derive(Serialize)]
struct ErrorBody {
    error: String,
}

/// An error the handler can return; axum turns it into a response
struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    /// A 422: valid JSON that breaks a contract rule
    fn unprocessable(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorBody {
                error: self.message,
            }),
        )
            .into_response()
    }
}

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

#[tokio::main]
async fn main() {
    let app = Router::new().route("/decide", post(decide_handler));

    // Only one process can listen on a port, so a second worker on 9001 fails here.
    let listener = TcpListener::bind("127.0.0.1:9001")
        .await
        .expect("Port already in use: try another one");

    axum::serve(listener, app).await.unwrap();
}

/// `POST /decide`: parse, validate, then build a fake decision.
async fn decide_handler(
    // Taking a Result instead of plain `Json<...>` means axum always calls this
    // handler, even when parsing fails, so its 400/415/422 rejections can be
    // reformatted as contract JSON instead of axum's plain-text default
    payload: Result<Json<DecideRequest>, JsonRejection>,
) -> Result<Json<DecideResponse>, ApiError> {
    let start = Instant::now();
    match payload {
        Ok(Json(req)) => {
            validate(&req)?;

            // Every question type becomes one list of options in declared
            // order, so the rest of the handler doesn't care which type it was
            let options = options_in_order(&req.question);

            // Stage 2: compute normalised hash weights for each option
            let mut total = 0.0;
            for option in &options {
                let w = weight(&req.input, option);
                total += w;
            }
            let mut probabilities: HashMap<String, f64> = HashMap::new();
            for option in &options {
                // called weight again instead of storing values in the previous for loop
                // to avoid creating a separate Vec to keep in step with options
                probabilities.insert(option.clone(), weight(&req.input, option) / total);
            }

            // simulate the model’s forward-pass cost (see the fake model parameters)
            let micros = BASE_MS
                + (COST_PER_BYTE * req.input.len() as u64)
                + (options.len() as u64 * COST_PER_OPTION);
            let duration = Duration::from_micros(micros);

            // Invalid requests are rejected before this point without sleeping,
            // as a real model server would reject them before running inference
            tokio::time::sleep(duration).await;

            // Safe to index [0]: validate guarantees at least 2 options.
            // Replace only on strictly greater, so the first option wins ties.
            // (Iterator::max_by would return the last one instead)
            let mut answer = &options[0];
            for option in &options {
                if probabilities[option] > probabilities[answer] {
                    answer = option;
                }
            }

            // Probability-weighted mean of the scale, for score only.
            // Keys are built from the integer so they match options_in_order
            let expected = match &req.question {
                Question::Score { min, max, .. } => {
                    let mut sum = 0.0;
                    for value in *min..=*max {
                        sum += value as f64 * probabilities[&value.to_string()];
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

            Ok(Json(DecideResponse {
                kind,
                expected,
                probabilities,
                answer: answer.clone(),
                model: "fake-v0".to_string(),
                latency_ms: start.elapsed().as_secs_f64() * 1000.0,
            }))
        }
        // Keep axum's status code and message; only the format changes
        Err(rejection) => Err(ApiError {
            status: rejection.status(),
            message: rejection.body_text(),
        }),
    }
}

// ---------------------------------------------------------------------------
// Validation: the semantic rules that types can't express.
// Each check is a guard that returns the first broken rule as a 422
// ---------------------------------------------------------------------------

fn validate(req: &DecideRequest) -> Result<(), ApiError> {
    // len() counts UTF-8 bytes, which is what the contract measures
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

            // Subtract in i16: two i8 values can differ by up to 255, which
            // overflows i8 (a panic in debug builds, a silent wrap in release)
            let width = i16::from(*max) - i16::from(*min);
            if width > 10 {
                return Err(ApiError::unprocessable(format!(
                    "score scale must have at most 11 points, got {}",
                    width + 1
                )));
            }

            if let Some(labels) = labels {
                // Safe cast: the checks above guarantee points is 2..=11
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

/// Rejects empty or duplicate entries. Shared by choice options and score
/// labels; `what` names the item in error messages
fn check_unique_non_empty(items: &[String], what: &str) -> Result<(), ApiError> {
    // insert() returns false if the value was already in the set
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

// ---------------------------------------------------------------------------
// Response helpers
// ---------------------------------------------------------------------------

/// The question's options in declared order, which decides ties
fn options_in_order(question: &Question) -> Vec<String> {
    match question {
        Question::Choice { options } => options.clone(),
        // Declared order is yes, then no, so a 50/50 tie answers "yes"
        Question::YesNo => vec!["yes".to_string(), "no".to_string()],
        Question::Score { min, max, .. } => (*min..=*max).map(|v| v.to_string()).collect(),
    }
}

// The normalised hash weights for the option
fn weight(input: &str, option: &str) -> f64 {
    let mut hasher = DefaultHasher::new();

    input.hash(&mut hasher);
    option.hash(&mut hasher);

    let n: u64 = hasher.finish();

    // Result is stable between versions, not guaranteed between different Rust verisons
    (n % 1000) as f64 + 1.0
}
