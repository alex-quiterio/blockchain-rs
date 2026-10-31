# 🔌 The infrastructure

The adapters: what plugs the [domain](../domain/README.md) and the [application](../application/README.md) into the real world. The inner layers declare the ports they need; each adapter here implements one, and `main.rs` wires them together. Nothing here decides a rule.

| Adapter | Implements | Where |
| --- | --- | --- |
| `ChainJsonRepository` | `ChainRepository` (domain) | `repository/json.rs` |
| `ChainInMemoryRepository` | `ChainRepository` (domain) | `repository/memory.rs` |
| `HttpPeerClient` | `PeerClient` (application) | `http.rs` |
| `log_event` | a `NodeEvent` listener | `logger.rs` |
| `Config` | a node's settings, from the environment | `config.rs` |

## Storage (`repository/`)

**`ChainJsonRepository`** keeps the chain in one file, one block of JSON per line.

- **Saving costs the same however long the chain gets.** The node passes the events of each change, and when they say the chain only grew, the new blocks are appended.
- **A fork rewrites the file atomically:** written to `<file>.tmp`, then renamed over the original, so a crash never leaves half a chain. The file is also rewritten whenever what's on disk isn't known for sure.
- **A torn last line is dropped on load.** A crash part way through an append leaves one, and that block was never confirmed as saved; peers send it again. The next save repairs the file.
- **Snapshots** of the balances go beside it, in `<file>.snapshot`, also written atomically. One that can't be read is no worse than none: the chain is replayed.
- **Older files**, one JSON array, still load, and are rewritten line by line on the next save.

**`ChainInMemoryRepository`** keeps the chain in memory, for tests and throwaway nodes. It shares the chain's blocks instead of copying them.

## Peers (`http.rs`)

**`HttpPeerClient`** talks to other nodes over the same REST API the web UI uses:

| `PeerClient` asks | Request |
| --- | --- |
| `status` | `GET /status` |
| `headers_from` | `GET /headers?from=N&limit=M` |
| `chain_from` | `GET /chain?from=N&limit=M` |
| `pending` | `GET /transactions/pending` |
| `offer_blocks` | `POST /chain` |
| `offer_transaction` | `POST /transactions` |
| `peers` | `GET /peers`, keeping those it reached and whose last attempt didn't fail |
| `introduce` | `POST /peers` |

- **Saying who's asking.** Status checks and offers carry this node's URL in the `x-peer` header. The peer then doesn't relay an offer straight back, and adds this node back after a status check if it had forgotten it.
- **Timeouts.** Every request times out after 3 seconds, so a dead peer can't hold up a sync.
- **Failures** come back as short strings for the network panel ("unreachable", "timed out", or the peer's status and message), never as panics.

## Logging (`logger.rs`)

`log_event` listens to the node and says in the log what happened, in the domain's words: a new chain started, blocks added, a switch to a heavier fork with how many transfers went back to the mempool, and transfers leaving the mempool. A transfer being accepted is logged only at `debug`, since there's one per transaction. `main.rs` subscribes it before the node opens, so it also hears that a new chain was started.

## Configuration (`config.rs`)

`Config::from_env` reads a node's settings from the environment: the chain rules, reorg limit, snapshot interval, mempool limit, address, chain file, public URL, peers, how many peers to discover, sync interval and how long to keep an unreachable peer. Every setting has a default, so a node starts with none set. A value that doesn't parse falls back to its default, except `CONSENSUS`, which must be `pow` or `pos`. The [main README](../../README.md) lists them all, under Configuration.

## Adding an adapter

Implement the port the inner layer declares, keep any rules out, and wire the adapter in `main.rs`. A new kind of storage implements `ChainRepository`, and `repository/json.rs`'s tests are a good list of what it should survive: torn writes, forks, snapshots and old formats.
