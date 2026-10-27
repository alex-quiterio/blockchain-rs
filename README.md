# blockchain-rs

A learning cryptocurrency in Rust with a REST API. It runs a single node on 127.0.0.1 and keeps everything in memory, so there is no real money involved.

- **Wallets** are ed25519 keypairs, and an address is the hex public key.
- **Transactions** are signed over `from|to|amount|nonce`. The nonce counts the sender's earlier transactions, which stops anyone replaying an old one.
- **Blocks** are linked by SHA-256 hashes and mined by proof of work: the hash has to start with `DIFFICULTY` zeros. The miner is paid `REWARD` coins through a coinbase transaction.
- **Validation** replays the whole chain and checks each block's link, hash, work, reward, signatures, nonces and balances.

## Run

```sh
cargo run                 # listens on 127.0.0.1:3000
DIFFICULTY=5 REWARD=25 BIND_ADDR=127.0.0.1:4000 cargo run
cargo test
```

## Front end

Open http://127.0.0.1:3000 while the node runs. It's plain HTML and JS in `public/`, served by the node itself. You can create named wallets, send coins between them, mine blocks, see the mempool and browse the chain, with a badge showing whether the chain is valid. Wallet keys are kept in the browser's localStorage.

## Endpoints

| Method | Path | What it does |
| --- | --- | --- |
| POST | `/wallets` | Make a keypair. Returns `address` and `secret_key`. |
| GET | `/balances/:address` | Confirmed balance and the next nonce, counting pending transactions. |
| POST | `/transactions/sign` | `{secret_key, to, amount, nonce?}` → a signed transaction, not submitted. |
| POST | `/transactions` | Submit a signed transaction to the mempool. |
| GET | `/transactions/pending` | The mempool. |
| POST | `/mine` | `{miner}`: mine the mempool into a block and pay the reward. |
| GET | `/chain` | Every block. |
| GET | `/chain/valid` | `{valid, length}` after a full replay. |

`/wallets` and `/transactions/sign` send secret keys over HTTP. That's a shortcut for experimenting locally. A real wallet signs on the client side and never sends its key.

## Try it

```sh
alice=$(curl -s -XPOST localhost:3000/wallets)
bob=$(curl -s -XPOST localhost:3000/wallets)
A=$(echo $alice | jq -r .address); B=$(echo $bob | jq -r .address)

curl -s -XPOST localhost:3000/mine -H 'content-type: application/json' -d "{\"miner\":\"$A\"}"
tx=$(curl -s -XPOST localhost:3000/transactions/sign -H 'content-type: application/json' \
  -d "{\"secret_key\":$(echo $alice | jq .secret_key),\"to\":\"$B\",\"amount\":20}")
curl -s -XPOST localhost:3000/transactions -H 'content-type: application/json' -d "$tx"
curl -s -XPOST localhost:3000/mine -H 'content-type: application/json' -d "{\"miner\":\"$B\"}"
curl -s localhost:3000/balances/$A    # 30
curl -s localhost:3000/balances/$B    # 70
```

## Layout

- `src/crypto.rs`: keys, signing, SHA-256
- `src/chain.rs`: ledger, mempool, mining, validation, plus the unit tests
- `src/api.rs`, `src/server.rs`: handlers and routes
- `tests/api_tests.rs`: HTTP round trips
- `public/`: the web UI

## Not built (yet)

- Peers and syncing between nodes
- Choosing the longest chain when nodes disagree
- Transaction fees
- Adjusting the difficulty over time
- Saving the chain to disk
