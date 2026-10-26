//! Keyless vending witness builder, proof runner, and bounded in-memory HTTP jobs.
use anyhow::{ensure, Result};
use async_trait::async_trait;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::Semaphore;
use vending_guest::Witness;

pub mod backend;
pub mod chain;
pub mod engine;
pub mod wire;

pub fn hex32(s: &str) -> Result<[u8; 32]> {
    ensure!(s.len() == 64, "expected 32-byte hex");
    hex::decode(s)?
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected 32-byte hex"))
}
pub fn pubkey(s: &str) -> Result<[u8; 32]> {
    bs58::decode(s)
        .into_vec()?
        .try_into()
        .map_err(|_| anyhow::anyhow!("expected 32-byte base58 public key"))
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ProveRequest {
    pub txid: String,
    pub machine: String,
    #[serde(alias = "nonce")]
    pub reservation_nonce: String,
}
impl ProveRequest {
    pub fn validate(&self) -> Result<()> {
        hex32(&self.txid)?;
        pubkey(&self.machine)?;
        hex32(&self.reservation_nonce)?;
        Ok(())
    }
}
#[derive(Clone, Debug, Deserialize)]
pub struct FindRequest {
    pub machine: String,
    #[serde(rename = "nonce", alias = "reservation_nonce")]
    pub reservation_nonce: String,
}
impl FindRequest {
    pub fn validate(&self) -> Result<()> {
        pubkey(&self.machine)?;
        hex32(&self.reservation_nonce)?;
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct FoundPayment {
    pub txid: String,
    pub height: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProofOutput {
    /// `execute` has no proof; `groth16` includes an SP1 v6 wire proof.
    pub mode: String,
    pub proof: Option<String>,
    pub public_values: String,
    pub cycles: u64,
    pub vkey_hash: Option<String>,
}
#[async_trait]
pub trait PaymentBackend: Send + Sync {
    async fn prepare(&self, request: &ProveRequest) -> Result<Witness>;
    async fn find(&self, request: &FindRequest) -> Result<Option<FoundPayment>>;
}
#[async_trait]
pub trait ProofEngine: Send + Sync {
    async fn run(&self, witness: Witness) -> Result<ProofOutput>;
}
#[derive(Clone, Serialize)]
struct Job {
    status: &'static str,
    result: Option<ProofOutput>,
    error: Option<String>,
    #[serde(skip)]
    created: Instant,
}
struct App {
    backend: Arc<dyn PaymentBackend>,
    engine: Arc<dyn ProofEngine>,
    jobs: Mutex<HashMap<String, Job>>,
    slots: Arc<Semaphore>,
    scan_slots: Arc<Semaphore>,
}
type HttpError = (StatusCode, Json<serde_json::Value>);
fn error(status: StatusCode, e: impl std::fmt::Display) -> HttpError {
    (status, Json(serde_json::json!({"error":e.to_string()})))
}

pub fn router(backend: Arc<dyn PaymentBackend>, engine: Arc<dyn ProofEngine>) -> Router {
    let state = Arc::new(App {
        backend,
        engine,
        jobs: Mutex::new(HashMap::new()),
        slots: Arc::new(Semaphore::new(1)),
        scan_slots: Arc::new(Semaphore::new(1)),
    });
    Router::new()
        .route("/prove", post(prove))
        .route("/jobs/{id}", get(job))
        .route("/find-payment", get(find))
        .with_state(state)
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(axum::extract::DefaultBodyLimit::max(2048))
}
async fn prove(
    State(app): State<Arc<App>>,
    Json(request): Json<ProveRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    request
        .validate()
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let permit = app
        .slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, "prover busy; retry"))?;
    let id = hex::encode(rand::random::<[u8; 16]>());
    {
        let mut jobs = app.jobs.lock().unwrap();
        jobs.retain(|_, j| j.created.elapsed() < Duration::from_secs(3600));
        if jobs.len() >= 128 {
            return Err(error(StatusCode::TOO_MANY_REQUESTS, "job capacity reached"));
        }
        jobs.insert(
            id.clone(),
            Job {
                status: "running",
                result: None,
                error: None,
                created: Instant::now(),
            },
        );
    }
    let task_id = id.clone();
    tokio::spawn(async move {
        let result = async {
            let witness = app.backend.prepare(&request).await?;
            app.engine.run(witness).await
        }
        .await;
        if let Some(job) = app.jobs.lock().unwrap().get_mut(&task_id) {
            match result {
                Ok(out) => {
                    job.status = "succeeded";
                    job.result = Some(out);
                }
                Err(e) => {
                    job.status = "failed";
                    job.error = Some(format!("{e:#}"));
                }
            }
        }
        drop(permit);
    });
    Ok((
        StatusCode::ACCEPTED,
        Json(serde_json::json!({"job_id":id,"status":"running"})),
    ))
}
async fn job(State(app): State<Arc<App>>, Path(id): Path<String>) -> Result<Json<Job>, HttpError> {
    app.jobs
        .lock()
        .unwrap()
        .get(&id)
        .filter(|j| j.created.elapsed() < Duration::from_secs(3600))
        .cloned()
        .map(Json)
        .ok_or_else(|| error(StatusCode::NOT_FOUND, "unknown or expired job"))
}
async fn find(
    State(app): State<Arc<App>>,
    Query(request): Query<FindRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), HttpError> {
    request
        .validate()
        .map_err(|e| error(StatusCode::BAD_REQUEST, e))?;
    let _permit = app
        .scan_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| error(StatusCode::TOO_MANY_REQUESTS, "scanner busy; retry"))?;
    match app
        .backend
        .find(&request)
        .await
        .map_err(|e| error(StatusCode::BAD_GATEWAY, e))?
    {
        Some(found) => Ok((StatusCode::OK, Json(serde_json::to_value(found).unwrap()))),
        None => Ok((
            StatusCode::ACCEPTED,
            Json(serde_json::json!({"status":"not_found"})),
        )),
    }
}
