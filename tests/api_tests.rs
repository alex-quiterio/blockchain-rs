use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use blockchain::chain::Blockchain;
use blockchain::server::{router, AppState};

fn app() -> Router {
    router(AppState::new(Blockchain::new(2, 50)))
}

async fn call(app: &Router, method: &str, uri: &str, body: Option<Value>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .unwrap();
    let response = app.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, value)
}

#[tokio::test]
async fn full_round_trip() {
    let app = app();

    let (status, alice) = call(&app, "POST", "/wallets", None).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_, bob) = call(&app, "POST", "/wallets", None).await;
    let alice_addr = alice["address"].as_str().unwrap();
    let bob_addr = bob["address"].as_str().unwrap();

    let (status, block) = call(&app, "POST", "/mine", Some(json!({ "miner": alice_addr }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(block["index"], 1);

    let (status, tx) = call(
        &app,
        "POST",
        "/transactions/sign",
        Some(json!({ "secret_key": alice["secret_key"], "to": bob_addr, "amount": 15 })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(tx["nonce"], 0);

    let (status, _) = call(&app, "POST", "/transactions", Some(tx.clone())).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    let (_, pending) = call(&app, "GET", "/transactions/pending", None).await;
    assert_eq!(pending.as_array().unwrap().len(), 1);

    call(&app, "POST", "/mine", Some(json!({ "miner": bob_addr }))).await;

    let (_, balance) = call(&app, "GET", &format!("/balances/{alice_addr}"), None).await;
    assert_eq!(balance["balance"], 35);
    assert_eq!(balance["next_nonce"], 1);
    let (_, balance) = call(&app, "GET", &format!("/balances/{bob_addr}"), None).await;
    assert_eq!(balance["balance"], 65);

    let (status, _) = call(&app, "POST", "/transactions", Some(tx)).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    let (_, validity) = call(&app, "GET", "/chain/valid", None).await;
    assert_eq!(validity, json!({ "valid": true, "length": 3 }));
}

#[tokio::test]
async fn rejects_tampered_transaction() {
    let app = app();
    let (_, alice) = call(&app, "POST", "/wallets", None).await;
    let (_, bob) = call(&app, "POST", "/wallets", None).await;
    call(&app, "POST", "/mine", Some(json!({ "miner": alice["address"] }))).await;

    let (_, mut tx) = call(
        &app,
        "POST",
        "/transactions/sign",
        Some(json!({ "secret_key": alice["secret_key"], "to": bob["address"], "amount": 1 })),
    )
    .await;
    tx["amount"] = json!(40);

    let (status, body) = call(&app, "POST", "/transactions", Some(tx)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["message"], "signature does not match sender");
}

#[tokio::test]
async fn rejects_bad_addresses() {
    let app = app();
    let (status, _) = call(&app, "GET", "/balances/not-hex", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(&app, "POST", "/mine", Some(json!({ "miner": "nobody" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
