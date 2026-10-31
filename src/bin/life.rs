//! A life ledger on top of a node: each trait of a life (health, work, love…) is an account, and
//! every entry about one is a transfer to it, from an account that stands for you.
//!
//!   NODE=http://127.0.0.1:3000 cargo run --bin life
//!
//! Settings, from the environment:
//!   NODE       the node to record on                    (default http://127.0.0.1:3000)
//!   LIFE_ADDR  where this API listens                   (127.0.0.1:4000)
//!   LIFE_FILE  the journal: keys and entries, in clear  (life.json)
//!
//! What an entry says never reaches the chain. The journal keeps the words; the transfer carries
//! only a `Memo`, the hash of the words with a random salt, so peers can't guess short entries
//! by hashing guesses. Once the transfer is in a block, the chain proves the entry was written by
//! then and hasn't been edited since: `GET /entries/:id` checks exactly that.
//!
//! An entry's weight is the coins it moves, 1 by default, so a trait's balance is how much of
//! you has gone into it. You pay from coins you mine: when you run short, an entry mines a
//! block to you first (proof of work only; under proof of stake, fund `GET /me`'s address).
//!
//!   GET  /me                      your address and balance
//!   GET  /traits                  each trait, its address, balance and number of entries
//!   POST /traits                  {"name": "garden"} starts a new one
//!   POST /traits/:name/entries    {"text": "…", "weight": 3} records an entry
//!   GET  /traits/:name/entries    a trait's entries, newest first, each with where it stands
//!   GET  /entries/:id             one entry, checked against the chain
//!   POST /seal                    mines a block, confirming whatever is waiting

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Utc};
use ed25519_dalek::SigningKey;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use blockchain::api::dto::transactions::TransactionResponse;
use blockchain::api::dto::wallets::BalanceResponse;
use blockchain::domain::accounting::transaction::Transaction;
use blockchain::domain::accounting::{Coins, Memo};
use blockchain::domain::crypto::{self, Address};

/// The traits a new journal starts with.
const STARTING_TRAITS: [&str; 9] =
    ["health", "work", "love", "friends", "family", "mind", "craft", "money", "home"];

struct Config {
    node: String,
    addr: String,
    file: PathBuf,
}

impl Config {
    fn from_env() -> Self {
        let var = |key: &str, default: &str| std::env::var(key).unwrap_or_else(|_| default.into());
        Self {
            node: var("NODE", "http://127.0.0.1:3000").trim_end_matches('/').to_string(),
            addr: var("LIFE_ADDR", "127.0.0.1:4000"),
            file: var("LIFE_FILE", "life.json").into(),
        }
    }
}

/// Everything this API keeps for itself: the keys, and the words the chain only has hashes of.
#[derive(Serialize, Deserialize)]
struct Journal {
    /// The account entries are paid from.
    me: String,
    /// Each trait's secret key, by name.
    traits: BTreeMap<String, String>,
    entries: Vec<Entry>,
}

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    id: u64,
    #[serde(rename = "trait")]
    trait_name: String,
    text: String,
    weight: u64,
    at: DateTime<Utc>,
    /// Random, so nobody can find a short entry by hashing guesses at it.
    salt: String,
    memo: Memo,
    /// The transfer's signature, which is how the node finds it.
    signature: String,
}

impl Entry {
    /// What the memo is the hash of. Everything that makes the entry what it is goes in, so
    /// changing any of it is caught.
    fn memo_of(trait_name: &str, text: &str, weight: u64, at: DateTime<Utc>, salt: &str) -> Memo {
        let at = at.to_rfc3339();
        Memo::of(format!("{salt}|{trait_name}|{at}|{weight}|{text}").as_bytes())
    }

    /// Whether the words kept here still hash to the memo.
    fn is_unedited(&self) -> bool {
        Self::memo_of(&self.trait_name, &self.text, self.weight, self.at, &self.salt) == self.memo
    }
}

impl Journal {
    fn new() -> Self {
        let traits = STARTING_TRAITS.iter().map(|name| (name.to_string(), new_key())).collect();
        Self { me: new_key(), traits, entries: Vec::new() }
    }

    fn open(file: &PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        match std::fs::read_to_string(file) {
            Ok(json) => Ok(serde_json::from_str(&json)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Written whole to a temporary file and moved into place, so a crash never leaves half.
    fn save(&self, file: &PathBuf) -> Result<(), LifeError> {
        let json = serde_json::to_string_pretty(self).expect("journals serialize");
        let tmp = file.with_extension("json.tmp");
        std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, file)).map_err(|e| {
            LifeError(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't save the journal: {e}"))
        })
    }

    fn me(&self) -> SigningKey {
        key(&self.me)
    }

    fn trait_key(&self, name: &str) -> Result<SigningKey, LifeError> {
        let secret = self.traits.get(name).ok_or_else(|| {
            LifeError(
                StatusCode::NOT_FOUND,
                format!("no trait named {name}; POST /traits to start one"),
            )
        })?;
        Ok(key(secret))
    }
}

fn new_key() -> String {
    crypto::generate_keypair().1
}

fn key(secret: &str) -> SigningKey {
    crypto::parse_secret_key(secret).expect("the journal only holds keys it made")
}

struct LifeError(StatusCode, String);

impl IntoResponse for LifeError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "message": self.1 }))).into_response()
    }
}

type Answer<T> = Result<Json<T>, LifeError>;

#[derive(Clone)]
struct Life {
    node: String,
    file: Arc<PathBuf>,
    http: Client,
    /// Held across a whole entry, so two never take the same nonce.
    journal: Arc<Mutex<Journal>>,
}

impl Life {
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, LifeError> {
        let response = self.http.get(format!("{}{path}", self.node)).send().await;
        Self::read(response).await
    }

    async fn post<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, LifeError> {
        let response = self.http.post(format!("{}{path}", self.node)).json(body).send().await;
        Self::read(response).await
    }

    /// The node's answer, or its refusal passed on with the same status.
    async fn read<T: serde::de::DeserializeOwned>(
        response: reqwest::Result<reqwest::Response>,
    ) -> Result<T, LifeError> {
        let unreachable =
            |e: reqwest::Error| LifeError(StatusCode::BAD_GATEWAY, format!("node: {e}"));
        let response = response.map_err(unreachable)?;
        let status =
            StatusCode::from_u16(response.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
        if !status.is_success() {
            let body: Value = response.json().await.unwrap_or_default();
            let message = body["message"].as_str().unwrap_or("refused").to_string();
            return Err(LifeError(status, format!("node: {message}")));
        }
        response.json().await.map_err(unreachable)
    }

    async fn balance(&self, address: &Address) -> Result<BalanceResponse, LifeError> {
        self.get(&format!("/balances/{address}")).await
    }

    async fn seal(&self, me: &Address) -> Result<Value, LifeError> {
        self.post("/mine", &json!({ "miner": me })).await
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().init();
    let config = Config::from_env();
    let journal = Journal::open(&config.file)?;
    journal.save(&config.file).map_err(|e| e.1)?;
    tracing::info!(
        "Life ledger on {} for {}, journal in {}: {} traits, {} entries",
        config.addr,
        config.node,
        config.file.display(),
        journal.traits.len(),
        journal.entries.len()
    );
    let life = Life {
        node: config.node,
        file: Arc::new(config.file),
        http: Client::new(),
        journal: Arc::new(Mutex::new(journal)),
    };
    let app = Router::new()
        .route("/me", get(me))
        .route("/traits", get(traits).post(add_trait))
        .route("/traits/:name/entries", get(trait_entries).post(record))
        .route("/entries/:id", get(entry))
        .route("/seal", post(seal))
        .with_state(life);
    let listener = tokio::net::TcpListener::bind(&config.addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn me(State(life): State<Life>) -> Answer<BalanceResponse> {
    let me = Address::of(&life.journal.lock().await.me());
    Ok(Json(life.balance(&me).await?))
}

#[derive(Serialize)]
struct TraitView {
    name: String,
    address: Address,
    /// How much of you has gone into it, counting confirmed entries only.
    balance: Coins,
    entries: usize,
}

async fn traits(State(life): State<Life>) -> Answer<Vec<TraitView>> {
    let journal = life.journal.lock().await;
    let mut views = Vec::new();
    for (name, secret) in &journal.traits {
        let address = Address::of(&key(secret));
        let balance = life.balance(&address).await?.balance;
        let entries = journal.entries.iter().filter(|e| &e.trait_name == name).count();
        views.push(TraitView { name: name.clone(), address, balance, entries });
    }
    Ok(Json(views))
}

#[derive(Deserialize)]
struct NewTrait {
    name: String,
}

async fn add_trait(
    State(life): State<Life>,
    Json(new): Json<NewTrait>,
) -> Result<(StatusCode, Json<Value>), LifeError> {
    let name = new.name.trim().to_lowercase();
    let well_formed = !name.is_empty()
        && name.len() <= 32
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !well_formed {
        let message = "a trait's name is 1 to 32 letters, digits or dashes";
        return Err(LifeError(StatusCode::BAD_REQUEST, message.into()));
    }
    let mut journal = life.journal.lock().await;
    if journal.traits.contains_key(&name) {
        return Err(LifeError(StatusCode::CONFLICT, format!("{name} is already a trait")));
    }
    let secret = new_key();
    let address = Address::of(&key(&secret));
    journal.traits.insert(name.clone(), secret);
    journal.save(&life.file)?;
    Ok((StatusCode::CREATED, Json(json!({ "name": name, "address": address }))))
}

#[derive(Deserialize)]
struct NewEntry {
    text: String,
    #[serde(default = "one")]
    weight: u64,
}

fn one() -> u64 {
    1
}

async fn record(
    State(life): State<Life>,
    Path(name): Path<String>,
    Json(new): Json<NewEntry>,
) -> Result<(StatusCode, Json<EntryView>), LifeError> {
    if new.text.trim().is_empty() || new.weight == 0 {
        let message = "an entry needs some text and a weight of at least 1";
        return Err(LifeError(StatusCode::BAD_REQUEST, message.into()));
    }
    let mut journal = life.journal.lock().await;
    let to = Address::of(&journal.trait_key(&name)?);
    let me = journal.me();
    let my_address = Address::of(&me);
    let weight = Coins::new(new.weight);
    // Coins sent but not yet in a block still count against the balance the node reports, so
    // an entry may find you short after all: then mine and try once more.
    let mut account = life.balance(&my_address).await?;
    if account.balance < weight {
        life.seal(&my_address).await?;
        account = life.balance(&my_address).await?;
    }
    let (at, salt) = (Utc::now(), hex::encode(rand::random::<[u8; 16]>()));
    let memo = Entry::memo_of(&name, &new.text, new.weight, at, &salt);
    let sign = |nonce| Transaction::with_memo(&me, &to, weight, nonce, Coins::ZERO, memo.clone());
    let tx = match life.post::<Transaction>("/transactions", &sign(account.next_nonce)).await {
        Err(LifeError(_, message)) if message.contains("balance") => {
            life.seal(&my_address).await?;
            let nonce = life.balance(&my_address).await?.next_nonce;
            life.post("/transactions", &sign(nonce)).await?
        }
        sent => sent?,
    };
    let entry = Entry {
        id: journal.entries.last().map_or(1, |e| e.id + 1),
        trait_name: name,
        text: new.text,
        weight: new.weight,
        at,
        salt,
        memo,
        signature: tx.signature().expect("transfers are signed").to_string(),
    };
    journal.entries.push(entry.clone());
    journal.save(&life.file)?;
    let view = EntryView { entry, chain: Standing::pending() };
    Ok((StatusCode::CREATED, Json(view)))
}

#[derive(Serialize)]
struct EntryView {
    #[serde(flatten)]
    entry: Entry,
    chain: Standing,
}

/// Where an entry's transfer stands on the chain, and whether the words still match it.
#[derive(Serialize)]
struct Standing {
    /// `confirmed`, `pending`, or `missing` when the node doesn't know the transfer.
    status: String,
    height: Option<u64>,
    sealed_at: Option<DateTime<Utc>>,
    confirmations: Option<u64>,
    /// The words kept here hash to the memo the transfer carries: nothing was edited.
    intact: Option<bool>,
}

impl Standing {
    fn pending() -> Self {
        let status = "pending".to_string();
        Self { status, height: None, sealed_at: None, confirmations: None, intact: Some(true) }
    }

    async fn of(life: &Life, entry: &Entry) -> Result<Self, LifeError> {
        let path = format!("/transactions/{}", entry.signature);
        let located = match life.get::<TransactionResponse>(&path).await {
            Ok(located) => located,
            Err(LifeError(StatusCode::NOT_FOUND, _)) => {
                let status = "missing".to_string();
                return Ok(Self {
                    status,
                    height: None,
                    sealed_at: None,
                    confirmations: None,
                    intact: None,
                });
            }
            Err(e) => return Err(e),
        };
        let intact = entry.is_unedited() && located.transaction.memo() == Some(&entry.memo);
        Ok(Self {
            status: located.status,
            height: located.block.as_ref().map(|b| b.height),
            sealed_at: located.block.as_ref().map(|b| b.timestamp),
            confirmations: located.block.as_ref().map(|b| b.confirmations),
            intact: Some(intact),
        })
    }
}

async fn trait_entries(
    State(life): State<Life>,
    Path(name): Path<String>,
) -> Answer<Vec<EntryView>> {
    let entries: Vec<Entry> = {
        let journal = life.journal.lock().await;
        journal.trait_key(&name)?;
        journal.entries.iter().rev().filter(|e| e.trait_name == name).cloned().collect()
    };
    let mut views = Vec::new();
    for entry in entries {
        let chain = Standing::of(&life, &entry).await?;
        views.push(EntryView { entry, chain });
    }
    Ok(Json(views))
}

async fn entry(State(life): State<Life>, Path(id): Path<u64>) -> Answer<EntryView> {
    let entry = life.journal.lock().await.entries.iter().find(|e| e.id == id).cloned();
    let entry = entry.ok_or_else(|| LifeError(StatusCode::NOT_FOUND, format!("no entry {id}")))?;
    let chain = Standing::of(&life, &entry).await?;
    Ok(Json(EntryView { entry, chain }))
}

async fn seal(State(life): State<Life>) -> Answer<Value> {
    let me = Address::of(&life.journal.lock().await.me());
    Ok(Json(life.seal(&me).await?))
}
