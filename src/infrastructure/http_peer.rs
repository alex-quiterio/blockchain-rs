use std::time::Duration;

use reqwest::{Client, Response};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::application::network::{PeerClient, PeerResult, PeerTip, PEER_HEADER};
use crate::domain::block::{Block, Transaction};
use crate::domain::chain::Extension;

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

    /// Posts `body`, saying who it's from when `me` is known.
    async fn post(
        &self,
        url: String,
        body: impl Serialize,
        me: Option<&str>,
    ) -> Result<Response, String> {
        let mut request = self.http.post(&url).json(&body);
        if let Some(me) = me {
            request = request.header(PEER_HEADER, me);
        }
        checked(request.send().await.map_err(describe)?).await
    }
}

/// The parts of a peer's `GET /status` and `POST /chain` replies this client reads.
#[derive(Deserialize)]
struct Status {
    height: u64,
    tip: String,
    weight: String,
    pending: usize,
}

#[derive(Deserialize)]
struct Offered {
    replaced: bool,
    #[serde(default)]
    missing_parent: bool,
}

impl Default for HttpPeerClient {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerClient for HttpPeerClient {
    fn status<'a>(&'a self, peer: &'a str) -> PeerResult<'a, PeerTip> {
        Box::pin(async move {
            let status: Status = self.get(format!("{peer}/status")).await?;
            Ok(PeerTip {
                height: status.height,
                tip: status.tip,
                weight: status.weight.parse().map_err(|_| "weight is not a number")?,
                pending: status.pending,
            })
        })
    }

    fn chain_from<'a>(&'a self, peer: &'a str, height: usize) -> PeerResult<'a, Vec<Block>> {
        Box::pin(self.get(format!("{peer}/chain?from={height}")))
    }

    fn pending<'a>(&'a self, peer: &'a str) -> PeerResult<'a, Vec<Transaction>> {
        Box::pin(self.get(format!("{peer}/transactions/pending")))
    }

    fn offer_blocks<'a>(
        &'a self,
        peer: &'a str,
        blocks: &'a [Block],
        me: Option<&'a str>,
    ) -> PeerResult<'a, Extension> {
        Box::pin(async move {
            let response = self.post(format!("{peer}/chain"), blocks, me).await?;
            let offered: Offered = response.json().await.map_err(describe)?;
            Ok(match (offered.missing_parent, offered.replaced) {
                (true, _) => Extension::MissingParent,
                (_, adopted) => adopted.into(),
            })
        })
    }

    fn offer_transaction<'a>(
        &'a self,
        peer: &'a str,
        tx: &'a Transaction,
        me: Option<&'a str>,
    ) -> PeerResult<'a, ()> {
        Box::pin(async move { self.post(format!("{peer}/transactions"), tx, me).await.map(drop) })
    }

    fn introduce<'a>(&'a self, peer: &'a str, me: &'a str) -> PeerResult<'a, ()> {
        let body = serde_json::json!({ "url": me });
        Box::pin(async move { self.post(format!("{peer}/peers"), body, None).await.map(drop) })
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
