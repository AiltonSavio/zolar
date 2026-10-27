use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;
use vending_guest::Witness;
use vending_prover::{
    router, FindRequest, FoundPayment, PaymentBackend, ProofEngine, ProofOutput, ProveRequest,
};
struct Fake;
#[async_trait]
impl PaymentBackend for Fake {
    async fn prepare(&self, request: &ProveRequest) -> anyhow::Result<Witness> {
        request.validate()?;
        Ok(serde_json::from_str(include_str!(
            "../../vending-guest/fixtures/synthetic/valid.json"
        ))?)
    }
    async fn find(&self, request: &FindRequest) -> anyhow::Result<Option<FoundPayment>> {
        request.validate()?;
        Ok(Some(FoundPayment {
            txid: "ab".repeat(32),
            height: 4515934,
            value_zat: 100000,
            payout: bs58::encode([4; 32]).into_string(),
        }))
    }
}
#[async_trait]
impl ProofEngine for Fake {
    async fn run(&self, w: Witness) -> anyhow::Result<ProofOutput> {
        let pv = vending_guest::verify(&w).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Ok(ProofOutput {
            mode: "groth16".into(),
            proof: Some("aa".into()),
            public_values: hex::encode(pv.encode()),
            cycles: 42,
            vkey_hash: None,
        })
    }
}
fn machine() -> String {
    bs58::encode([2; 32]).into_string()
}
#[tokio::test]
async fn endpoints_return_a_job_then_values_and_scan_result() {
    let app = router(Arc::new(Fake), Arc::new(Fake));
    let body = serde_json::json!({"txid":"ab".repeat(32),"machine":machine(),"nonce":bs58::encode([3;32]).into_string()});
    let response = app
        .clone()
        .oneshot(
            Request::post("/prove")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let v: serde_json::Value =
        serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let id = v["job"].as_str().unwrap();
    let mut complete = false;
    for _ in 0..100 {
        let r = app
            .clone()
            .oneshot(
                Request::get(format!("/prove/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
        if v["status"] == "done" {
            assert_eq!(v["proof"], "aa");
            assert_eq!(v["public_values"].as_str().unwrap().len(), 498);
            complete = true;
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(complete);
    let r = app
        .oneshot(
            Request::get(format!(
                "/find-payment?machine={}&nonce={}",
                machine(),
                bs58::encode([3; 32]).into_string()
            ))
            .body(Body::empty())
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let v: serde_json::Value =
        serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    assert_eq!(v["height"], 4515934);
    assert_eq!(v["status"], "found");
    assert_eq!(v["value_zat"], 100000);
    assert_eq!(v["payout"], bs58::encode([4; 32]).into_string());
}
#[tokio::test]
async fn malformed_jobs_and_queries_are_rejected_before_work_is_scheduled() {
    let app = router(Arc::new(Fake), Arc::new(Fake));
    let body = serde_json::json!({"txid":"bad","machine":machine(),"nonce":bs58::encode([3;32]).into_string()});
    let r = app
        .clone()
        .oneshot(
            Request::post("/prove")
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    let r = app
        .oneshot(
            Request::get("/prove/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}

struct Gated {
    prepare: tokio::sync::Semaphore,
    prove: tokio::sync::Semaphore,
    entered: tokio::sync::Notify,
    execute: bool,
}
#[async_trait]
impl PaymentBackend for Gated {
    async fn prepare(&self, r: &ProveRequest) -> anyhow::Result<Witness> {
        self.prepare.acquire().await.unwrap().forget();
        Fake.prepare(r).await
    }
    async fn find(&self, _: &FindRequest) -> anyhow::Result<Option<FoundPayment>> {
        Ok(None)
    }
}
#[async_trait]
impl ProofEngine for Gated {
    async fn run(&self, w: Witness) -> anyhow::Result<ProofOutput> {
        self.entered.notify_one();
        self.prove.acquire().await.unwrap().forget();
        if self.execute {
            Ok(ProofOutput {
                mode: "execute".into(),
                proof: None,
                public_values: "00".repeat(249),
                cycles: 42,
                vkey_hash: None,
            })
        } else {
            Fake.run(w).await
        }
    }
}
async fn json(app: axum::Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
    let r = app.oneshot(request).await.unwrap();
    let code = r.status();
    let v = serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
    (code, v)
}
#[tokio::test]
async fn pending_and_job_phases_match_contract_and_execute_never_returns_a_proof() {
    let backend = Arc::new(Gated {
        prepare: tokio::sync::Semaphore::new(0),
        prove: tokio::sync::Semaphore::new(0),
        entered: tokio::sync::Notify::new(),
        execute: true,
    });
    let app = router(backend.clone(), backend.clone());
    let body = serde_json::json!({"txid":"ab".repeat(32),"machine":machine(),"nonce":bs58::encode([3;32]).into_string()});
    let post = || {
        Request::post("/prove")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let (code, created) = json(app.clone(), post()).await;
    assert_eq!(code, StatusCode::ACCEPTED);
    assert_eq!(created.as_object().unwrap().len(), 1);
    let url = format!("/prove/{}", created["job"].as_str().unwrap());
    let poll = || Request::get(&url).body(Body::empty()).unwrap();
    let (_, waiting) = json(app.clone(), poll()).await;
    assert_eq!(waiting, serde_json::json!({"status":"waiting_finality"}));
    assert_eq!(
        json(app.clone(), post()).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    backend.prepare.add_permits(1);
    backend.entered.notified().await;
    assert_eq!(
        json(app.clone(), poll()).await.1,
        serde_json::json!({"status":"proving"})
    );
    backend.prove.add_permits(1);
    let mut failed = None;
    for _ in 0..100 {
        tokio::task::yield_now().await;
        let (_, state) = json(app.clone(), poll()).await;
        if state["status"] == "failed" {
            failed = Some(state);
            break;
        }
    }
    let failed = failed.expect("execute-only job must finish as failed");
    assert!(failed["error"]
        .as_str()
        .unwrap()
        .contains("without a claimable proof"));
    assert!(failed.get("proof").is_none());
    let (code, pending) = json(
        app,
        Request::get(format!(
            "/find-payment?machine={}&nonce={}",
            machine(),
            bs58::encode([3; 32]).into_string()
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(pending, serde_json::json!({"status":"pending"}));
}

#[test]
fn nonce_requires_canonical_base58_of_exactly_32_bytes() {
    let make = |nonce: String| ProveRequest {
        txid: "ab".repeat(32),
        machine: machine(),
        nonce,
    };
    assert!(make(bs58::encode([3; 32]).into_string()).validate().is_ok());
    for nonce in [
        "03".repeat(32),
        "1".repeat(31),
        "1".repeat(33),
        format!(" {}", bs58::encode([3; 32]).into_string()),
    ] {
        assert!(make(nonce).validate().is_err());
    }
}

struct Finalizing(std::sync::atomic::AtomicUsize);
#[async_trait]
impl PaymentBackend for Finalizing {
    async fn prepare(&self, r: &ProveRequest) -> anyhow::Result<Witness> {
        if self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            Err(vending_prover::WaitingFinality.into())
        } else {
            Fake.prepare(r).await
        }
    }
    async fn find(&self, _: &FindRequest) -> anyhow::Result<Option<FoundPayment>> {
        Ok(None)
    }
}
#[tokio::test(start_paused = true)]
async fn a_job_waits_and_retries_when_payment_has_not_reached_finality() {
    let backend = Arc::new(Finalizing(std::sync::atomic::AtomicUsize::new(0)));
    let app = router(backend.clone(), Arc::new(Fake));
    let body = serde_json::json!({"txid":"ab".repeat(32),"machine":machine(),"nonce":bs58::encode([3;32]).into_string()});
    let (_, created) = json(
        app.clone(),
        Request::post("/prove")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
    tokio::task::yield_now().await;
    let url = format!("/prove/{}", created["job"].as_str().unwrap());
    let poll = || Request::get(&url).body(Body::empty()).unwrap();
    assert_eq!(
        json(app.clone(), poll()).await.1["status"],
        "waiting_finality"
    );
    tokio::time::advance(std::time::Duration::from_secs(5)).await;
    for _ in 0..100 {
        tokio::task::yield_now().await;
        if json(app.clone(), poll()).await.1["status"] == "done" {
            assert_eq!(backend.0.load(std::sync::atomic::Ordering::SeqCst), 2);
            return;
        }
    }
    panic!("job did not resume after finality");
}
