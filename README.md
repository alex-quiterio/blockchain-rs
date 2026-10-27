# blockchain-rs

A learning cryptocurrency in Rust with a REST API. Each node runs on 127.0.0.1 and saves its chain to a JSON file. There is no real money involved.

- **Wallets** are ed25519 keypairs, and an address is the hex public key.
- **Transactions** are signed over `from|to|amount|nonce`. The nonce counts the sender's earlier transactions, which stops anyone replaying an old one.
- **Blocks** are linked by SHA-256 hashes and mined by proof of work: the hash has to start with `DIFFICULTY` zeros. The miner is paid `REWARD` coins through a coinbase transaction.
- **Validation** replays the whole chain and checks each block's link, hash, work, reward, signatures, nonces and balances.
- **Storage**: after every new block the chain is written to `CHAIN_FILE` (default `chain.json`) and replayed in full on startup, so a tampered file is refused. The mempool is not saved.
- **Longest chain wins**: offer a node another node's chain and it switches if that chain is valid and longer. Transfers that only its old blocks had confirmed go back to its mempool.

## Run

```sh
cargo run                 # listens on 127.0.0.1:3000
DIFFICULTY=5 REWARD=25 BIND_ADDR=127.0.0.1:4000 CHAIN_FILE=node2.json cargo run
cargo test
```

A saved chain only loads with the `DIFFICULTY` and `REWARD` it was mined with. Delete the file to start over.

CI (`.github/workflows/test.yml`) runs `cargo fmt --check`, `cargo clippy` and `cargo test` on every push to `main` and every pull request.

## Two nodes that disagree

There's no peer discovery yet, so you hand one node's chain to the other yourself:

```sh
curl -s localhost:4000/chain | curl -s -XPOST localhost:3000/chain -H 'content-type: application/json' -d @-
# {"replaced":true,"length":4}   or   {"replaced":false,...} if 3000's chain was as long
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
| POST | `/chain` | Offer another node's blocks. Adopted only if valid and longer: `{replaced, length}`. |
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

The code follows domain-driven design. Each layer only depends on the ones above it in this list.

- `src/domain/`: the core rules, with no I/O.
  - `chain.rs`: the `Blockchain` aggregate root (mining, validation, `adopt_if_longer`)
  - `block.rs`: `Block` (mines and hashes itself) and `Transaction`
  - `mempool.rs`: transfers waiting to be mined, admitted only if they apply after the rest of the queue
  - `ledger.rs`: balances and nonces
  - `rules.rs`: `ChainRules`, the difficulty and reward every node must share
  - `address.rs`: `Address`, a value object that can only hold a valid public key
  - `crypto.rs`: keys, signing, SHA-256
  - `repository.rs`: the `ChainRepository` port
- `src/application/`: `NodeService` runs each use case. A change is made on a copy of the chain and only goes live once it's saved, so memory and disk never disagree.
- `src/infrastructure/`: `JsonFileRepository` (atomic writes via a temp file and rename) and `InMemoryRepository`
- `src/api/`: HTTP routes, handlers and request/response types
- `tests/api_tests.rs`: HTTP round trips; unit tests sit next to the code they cover
- `public/`: the web UI

## Not built (yet)

- Peers and syncing between nodes: chains are only exchanged by hand through `POST /chain`
- Transaction fees
- Adjusting the difficulty over time
