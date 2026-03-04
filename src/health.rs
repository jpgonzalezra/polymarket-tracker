use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde_json::json;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct HealthState {
    pub pool: PgPool,
    pub last_poll: Arc<RwLock<Option<std::time::Instant>>>,
    pub poll_interval: Duration,
}

pub fn router(state: HealthState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .with_state(state)
}

async fn healthz() -> impl IntoResponse {
    Json(json!({"status": "ok"}))
}

async fn readyz(State(state): State<HealthState>) -> impl IntoResponse {
    // Check DB
    let db_ok = sqlx::query("SELECT 1")
        .execute(&state.pool)
        .await
        .is_ok();

    // Check last poll is recent
    let poll_ok = {
        let lp = state.last_poll.read().await;
        match *lp {
            Some(t) => t.elapsed() < state.poll_interval * 2,
            None => false, // Never polled yet — not ready
        }
    };

    if db_ok && poll_ok {
        (StatusCode::OK, Json(json!({"status": "ready", "db": true, "poll": true})))
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "not ready", "db": db_ok, "poll": poll_ok})),
        )
    }
}
