use axum::{
    body::Body,
    http::{Request, StatusCode},
    Router,
};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

use blockchain::api::{router, AppState};
use std::sync::Arc;

use blockchain::api::dto::ProofResponse;
use blockchain::application::{Network, NodeService};
use blockchain::domain::block::BlockHeader;
use blockchain::domain::merkle;
use blockchain::domain::rules::ChainRules;
use blockchain::domain::values::Coins;
use blockchain::infrastructure::{HttpPeerClient, InMemoryRepository};

fn app() -> Router {
    app_with(ChainRules::proof_of_work(2, 50))
}

fn app_with(rules: ChainRules) -> Router {
    let node = NodeService::open(Box::new(InMemoryRepository::default()), rules).unwrap();
    router(AppState::new(Network::new(Arc::new(node), Box::new(HttpPeerClient::new()), None)))
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
    assert_eq!(validity, json!({ "valid": true, "length": 3, "weight": "512" }));
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

#[tokio::test]
async fn nodes_settle_on_the_heaviest_chain() {
    let (ours, theirs) = (app(), app());
    let (_, miner) = call(&ours, "POST", "/wallets", None).await;
    let miner = json!({ "miner": miner["address"] });
    call(&ours, "POST", "/mine", Some(miner.clone())).await;
    call(&theirs, "POST", "/mine", Some(miner.clone())).await;
    call(&theirs, "POST", "/mine", Some(miner)).await;

    let (_, their_chain) = call(&theirs, "GET", "/chain", None).await;
    let (status, body) = call(&ours, "POST", "/chain", Some(their_chain.clone())).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, json!({ "replaced": true, "length": 3, "missing_parent": false }));
    let (_, our_chain) = call(&ours, "GET", "/chain", None).await;
    assert_eq!(our_chain, their_chain);

    // Offering our now-equal chain back changes nothing.
    let (_, body) = call(&theirs, "POST", "/chain", Some(our_chain)).await;
    assert_eq!(body, json!({ "replaced": false, "length": 3, "missing_parent": false }));
}

#[tokio::test]
async fn a_node_takes_new_blocks_alone_and_asks_for_more_when_it_must() {
    let (ours, theirs) = (app(), app());
    let (_, miner) = call(&ours, "POST", "/wallets", None).await;
    let miner = json!({ "miner": miner["address"] });
    for _ in 0..3 {
        call(&theirs, "POST", "/mine", Some(miner.clone())).await;
    }

    let (_, tip) = call(&theirs, "GET", "/chain?from=3", None).await;
    let (_, body) = call(&ours, "POST", "/chain", Some(tip.clone())).await;
    assert_eq!(body, json!({ "replaced": false, "length": 1, "missing_parent": true }));

    let (_, recent) = call(&theirs, "GET", "/chain?from=1", None).await;
    let (_, body) = call(&ours, "POST", "/chain", Some(recent)).await;
    assert_eq!(body, json!({ "replaced": true, "length": 4, "missing_parent": false }));
    let (_, body) = call(&ours, "POST", "/chain", Some(tip)).await;
    assert_eq!(body, json!({ "replaced": false, "length": 4, "missing_parent": false }));
}

#[tokio::test]
async fn refuses_a_forged_heavier_chain() {
    let (ours, theirs) = (app(), app());
    let (_, miner) = call(&ours, "POST", "/wallets", None).await;
    let miner = json!({ "miner": miner["address"] });
    call(&theirs, "POST", "/mine", Some(miner.clone())).await;
    call(&theirs, "POST", "/mine", Some(miner)).await;

    let (_, mut chain) = call(&theirs, "GET", "/chain", None).await;
    chain[2]["transactions"][0]["amount"] = json!(1_000_000);
    let (status, body) = call(&ours, "POST", "/chain", Some(chain)).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "chain does not validate");
}

#[tokio::test]
async fn proof_of_stake_round_trip() {
    let app = app_with(ChainRules::proof_of_stake(50));
    let (_, alice) = call(&app, "POST", "/wallets", None).await;
    let (_, bob) = call(&app, "POST", "/wallets", None).await;
    let alice_key = json!({ "secret_key": alice["secret_key"] });

    let (status, _) = call(&app, "POST", "/mine", Some(json!({ "miner": alice["address"] }))).await;
    assert_eq!(status, StatusCode::CONFLICT);

    let (status, block) = call(&app, "POST", "/forge", Some(alice_key.clone())).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(block["validator"], alice["address"]);

    let (_, tx) = call(
        &app,
        "POST",
        "/transactions/sign",
        Some(json!({ "secret_key": alice["secret_key"], "to": "stake", "amount": 40, "fee": 2 })),
    )
    .await;
    let (status, _) = call(&app, "POST", "/transactions", Some(tx)).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    call(&app, "POST", "/forge", Some(json!({ "secret_key": bob["secret_key"] }))).await;

    let (_, consensus) = call(&app, "GET", "/consensus", None).await;
    assert_eq!(consensus["kind"], "proof_of_stake");
    assert_eq!(consensus["next_validator"], alice["address"]);
    assert_eq!(consensus["total_stake"], 40);

    let (status, body) =
        call(&app, "POST", "/forge", Some(json!({ "secret_key": bob["secret_key"] }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body["message"].as_str().unwrap().contains("to forge"));

    let (_, balance) =
        call(&app, "GET", &format!("/balances/{}", alice["address"].as_str().unwrap()), None).await;
    assert_eq!(balance["balance"], 8);
    assert_eq!(balance["staked"], 40);
    let (_, balance) =
        call(&app, "GET", &format!("/balances/{}", bob["address"].as_str().unwrap()), None).await;
    assert_eq!(balance["balance"], 52, "bob forged the block holding alice's fee");
}

#[tokio::test]
async fn refuses_to_rewrite_history_deeper_than_allowed() {
    let rules = ChainRules::proof_of_work(2, 50).with_max_reorg_depth(1);
    let (ours, theirs) = (app_with(rules), app_with(rules));
    let (_, miner) = call(&ours, "POST", "/wallets", None).await;
    let miner = json!({ "miner": miner["address"] });
    for _ in 0..2 {
        call(&ours, "POST", "/mine", Some(miner.clone())).await;
    }
    for _ in 0..4 {
        call(&theirs, "POST", "/mine", Some(miner.clone())).await;
    }

    let (_, chain) = call(&theirs, "GET", "/chain", None).await;
    let (status, body) = call(&ours, "POST", "/chain", Some(chain)).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["message"], "switching forks would undo 2 blocks; at most 1 may be undone");
}

#[tokio::test]
async fn forging_with_a_key_that_never_leaves_the_client() {
    use blockchain::domain::{block::Block, crypto};

    let app = app_with(ChainRules::proof_of_stake(50));
    let (address, secret) = crypto::generate_keypair();
    let key = crypto::parse_secret_key(&secret).unwrap();

    let (status, proposal) =
        call(&app, "POST", "/forge/proposal", Some(json!({ "validator": address }))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(proposal.get("signature").is_none());

    let mut block: Block = serde_json::from_value(proposal).unwrap();
    block.sign(&key);
    let (status, body) =
        call(&app, "POST", "/blocks", Some(serde_json::to_value(&block).unwrap())).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let (status, _) =
        call(&app, "POST", "/blocks", Some(serde_json::to_value(&block).unwrap())).await;
    assert_eq!(status, StatusCode::CONFLICT, "the same block again is stale");
    let (_, balance) = call(&app, "GET", &format!("/balances/{address}"), None).await;
    assert_eq!(balance["balance"], 50);
}

#[tokio::test]
async fn a_light_client_checks_a_transfer_from_headers_and_a_proof() {
    let app = app();
    let (_, alice) = call(&app, "POST", "/wallets", None).await;
    let (_, bob) = call(&app, "POST", "/wallets", None).await;
    call(&app, "POST", "/mine", Some(json!({ "miner": alice["address"] }))).await;
    let mut signatures = Vec::new();
    for amount in 1..=3 {
        let request =
            json!({ "secret_key": alice["secret_key"], "to": bob["address"], "amount": amount });
        let (_, tx) = call(&app, "POST", "/transactions/sign", Some(request)).await;
        call(&app, "POST", "/transactions", Some(tx.clone())).await;
        signatures.push(tx["signature"].as_str().unwrap().to_string());
    }
    let uri = format!("/transactions/{}/proof", signatures[1]);
    let (status, _) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "still waiting in the mempool");
    call(&app, "POST", "/mine", Some(json!({ "miner": alice["address"] }))).await;

    let (status, body) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    let proof: ProofResponse = serde_json::from_value(body).unwrap();
    assert_eq!(proof.transaction.amount, Coins::new(2));
    assert_eq!(proof.leaf, merkle::leaf(&proof.transaction));
    assert!(merkle::verify(&proof.leaf, &proof.proof, &proof.header.merkle_root));

    // The light client holds only headers, and the proof's header is one of them.
    let (_, headers) = call(&app, "GET", "/headers", None).await;
    let headers: Vec<BlockHeader> = serde_json::from_value(headers).unwrap();
    assert_eq!(headers.len(), 3);
    assert!(headers.iter().all(|h| h.hash == h.computed_hash()));
    assert!(headers.windows(2).all(|pair| pair[1].prev_hash == pair[0].hash));
    assert_eq!(headers[2], proof.header);
}

#[tokio::test]
async fn the_explorer_finds_blocks_transactions_and_addresses() {
    let app = app();
    let (_, alice) = call(&app, "POST", "/wallets", None).await;
    let (_, bob) = call(&app, "POST", "/wallets", None).await;
    let (_, first) = call(&app, "POST", "/mine", Some(json!({ "miner": alice["address"] }))).await;
    let request =
        json!({ "secret_key": alice["secret_key"], "to": bob["address"], "amount": 4, "fee": 1 });
    let (_, tx) = call(&app, "POST", "/transactions/sign", Some(request)).await;
    call(&app, "POST", "/transactions", Some(tx.clone())).await;
    let uri = format!("/transactions/{}", tx["signature"].as_str().unwrap());

    let (_, found) = call(&app, "GET", &uri, None).await;
    assert_eq!((found["status"].clone(), found["block"].clone()), (json!("pending"), Value::Null));
    call(&app, "POST", "/mine", Some(json!({ "miner": alice["address"] }))).await;
    let (status, found) = call(&app, "GET", &uri, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found["status"], "confirmed");
    assert_eq!(found["transaction"], tx);
    assert_eq!(
        (found["block"]["height"].clone(), found["block"]["position"].clone()),
        (json!(2), json!(1))
    );

    let (_, by_height) = call(&app, "GET", "/blocks/1", None).await;
    assert_eq!(by_height, first);
    let (_, page) = call(&app, "GET", "/chain?from=1&limit=1", None).await;
    assert_eq!(page, json!([first]));
    let (_, page) = call(&app, "GET", "/chain?from=9&limit=5", None).await;
    assert_eq!(page, json!([]));
    let (_, works) = call(&app, "GET", "/chain/work?from=1&limit=1", None).await;
    assert_eq!(works.as_array().unwrap().len(), 1);
    let (_, works) = call(&app, "GET", "/chain/work?from=9", None).await;
    assert_eq!(works, json!([]));
    let (_, by_hash) =
        call(&app, "GET", &format!("/blocks/{}", first["hash"].as_str().unwrap()), None).await;
    assert_eq!(by_hash, first);
    let (status, _) = call(&app, "GET", "/blocks/99", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/transactions/nope", None).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = call(&app, "GET", "/transactions/pending", None).await;
    assert_eq!(status, StatusCode::OK, "the mempool route still wins");

    let uri = format!("/addresses/{}/transactions", bob["address"].as_str().unwrap());
    let (_, history) = call(&app, "GET", &uri, None).await;
    assert_eq!(history.as_array().unwrap().len(), 1);
    assert_eq!(history[0]["transaction"]["amount"], 4);
    let (status, _) = call(&app, "GET", "/addresses/not-hex/transactions", None).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}
