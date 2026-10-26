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
        }))
    }
}
#[async_trait]
impl ProofEngine for Fake {
    async fn run(&self, w: Witness) -> anyhow::Result<ProofOutput> {
        let pv = vending_guest::verify(&w).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        Ok(ProofOutput {
            mode: "execute".into(),
            proof: None,
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
    let body = serde_json::json!({"txid":"ab".repeat(32),"machine":machine(),"reservation_nonce":"03".repeat(32)});
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
    let id = v["job_id"].as_str().unwrap();
    let mut complete = false;
    for _ in 0..100 {
        let r = app
            .clone()
            .oneshot(
                Request::get(format!("/jobs/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v: serde_json::Value =
            serde_json::from_slice(&r.into_body().collect().await.unwrap().to_bytes()).unwrap();
        if v["status"] == "succeeded" {
            assert!(v["result"]["proof"].is_null());
            assert_eq!(v["result"]["mode"], "execute");
            assert_eq!(v["result"]["public_values"].as_str().unwrap().len(), 498);
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
                "03".repeat(32)
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
}
#[tokio::test]
async fn malformed_jobs_and_queries_are_rejected_before_work_is_scheduled() {
    let app = router(Arc::new(Fake), Arc::new(Fake));
    let body = serde_json::json!({"txid":"bad","machine":machine(),"nonce":"03".repeat(32)});
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
            Request::get("/jobs/nonexistent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
}
