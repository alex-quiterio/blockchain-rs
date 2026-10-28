use std::time::Duration;

use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::application::network::{PeerClient, PeerResult};
use crate::domain::block::{Block, Transaction};

/// Talks to other nodes over their REST API.
pub struct HttpPeerClient {
    http: Client,
}

impl HttpPeerClient {
    pub fn new() -> Self {
        let http = Client::builder().timeout(Duration::from_secs(3)).build().expect("http client");
        Self { http }
    }

    async fn get<T: DeserializeOwned>(&self, url: String) -> Result<T, String> {
        let response = self.http.get(&url).send().await.map_err(describe)?;
        checked(response).await?.json().await.map_err(describe)
    }

    async fn post(&self, url: String, body: impl Serialize) -> Result<(), String> {
        let response = self.http.post(&url).json(&body).send().await.map_err(describe)?;
        checked(response).await.map(drop)
    }
}

impl Default for HttpPeerClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerClient for HttpPeerClient {
    fn chain<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Block>> {
        Box::pin(self.get(format!("{peer}/chain")))
    }

    fn pending<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Transaction>> {
        Box::pin(self.get(format!("{peer}/transactions/pending")))
    }

    fn offer_chain<'a>(&'a self, peer: &'a str, blocks: &'a [Block]) -> PeerResult<'a, ()> {
        Box::pin(self.post(format!("{peer}/chain"), blocks))
    }

    fn offer_transaction<'a>(&'a self, peer: &'a str, tx: &'a Transaction) -> PeerResult<'a, ()> {
        Box::pin(self.post(format!("{peer}/transactions"), tx))
    }

    fn introduce<'a>(&'a self, peer: &'a str, me: &'a str) -> PeerResult<'a, ()> {
        Box::pin(self.post(format!("{peer}/peers"), serde_json::json!({ "url": me })))
    }
}

async fn checked(response: Response) -> Result<Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("{status}: {body}"))
}

fn describe(err: reqwest::Error) -> String {
    if err.is_connect() {
        "unreachable".to_string()
    } else if err.is_timeout() {
        "timed out".to_string()
    } else {
        err.to_string()
    }
}
