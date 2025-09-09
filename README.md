# Sqew

Sqew is a lightweight message queue service and cli tool.

## Project Goals
- Build a **lightweight, embeddable message queue service** with HTTP + JSON endpoints.
- Focus on **simplicity, reliability, and portability**: no external dependencies beyond SQLite.
- Provide **at-least-once delivery** with configurable visibility timeouts and retries.
- Keep the system **observable** (metrics, logs, health checks) and **operationally safe** (TTLs, cleanup).

---

## Requirements

### Functional
- **Queue Management**
  - Create and manage named queues.

- **Message Lifecycle**
  - Enqueue JSON payloads with optional delay.
  - Poll messages atomically (batch support).
  - Acknowledge (ack) processed messages → delete permanently.
  - Negative acknowledge (nack) → increment attempts and reschedule; messages exceeding the retry limit will be dropped.

- **APIs**
  - REST/HTTP endpoints with JSON.
  - Admin/debug endpoints (peek, stats, purge, requeue).

- **Delivery Semantics**
  - At-least-once delivery.
  - Message order is not guaranteed

- **Observability**
  - `/healthz` and `/readyz` endpoints.
  - Metrics endpoint (`/metrics` Prometheus format).
  - Structured logs and tracing.
  - Per-queue stats, error rates metrics should be included

- **Security**
  - Per-queue API keys.
  - Request size limits and strict JSON validation.
  - Optional HMAC signing of payloads.

### Non-Functional
- **Performance**
  - Single-writer safe with SQLite WAL mode.
  - Batch operations to reduce contention.
  - Designed for small to medium workloads (10^4 – 10^5 msgs/day).

- **Portability**
  - Single binary deployment.
  - SQLite file as sole state store.

- **Maintainability**
  - Modular Rust codebase (`axum`, `sqlx`, `serde`, `tracing`).
  - Schema managed with migrations.
  - Unit + integration tests (ephemeral DB).

---

# Core semantics

* Delivery guarantees: keep it **at-least-once**; 
* Ordering: SQLite can’t guarantee strict FIFO under concurrency. Use `(priority DESC, available_at, id)` ordering and document it as **best-effort ordering**.
* Retries: track **attempts** with **max\_attempts**, exponential backoff to **available\_at** on `nack`.
* Messages exceeding `max_attempts` will be dropped and not re-presented to consumers.
* Size limits: enforce max payload size (e.g. 512 KB JSON) to keep DB healthy.

---

## Specification

### CLI Usage

```bash
 sqew serve # Will host a new sqew service at port 8888 by default
 sqew --help # Displays help information
 sqew --version # Shows version information
 sqew queue list # List available queues
 sqew queue add --name {name} --max-attempts <max_attempts:5> # Adds a new queue
 sqew queue rm --name {name} # Delete a queue
 sqew queue show --name {name} # Show information and stats about the queue
 sqew queue purge --name {name} # Purge messages from queue
 sqew queue peek --name {name} --limit <n> # Peek N items from queue
 sqew message remove --queue {name} --id <id> # Remove messages from queue by id
 sqew message peek --queue {name} --id <id> # Peek message from queue by id
 sqew message get --queue {name} --batch <n> # Get message(s), default is 1>
 sqew message enqueue --queue {name} --file <json_file> --payload <json> # enqueue messages from a newline-delimited JSON file
 sqew message poll --queue {name} --batch <n> # poll up to n messages
 sqew message ack --queue {name} --ids <id1,id2,...> # acknowledge multiple messages
 sqew message nack --queue {name} --ids <id1,id2,...> [--delay-ms <ms>] # nack multiple messages
 sqew stats --queue {name} #(show queue stats)
 sqew health # (check service health)
 sqew metrics # (show metrics in Prometheus format)
```




# Backoff & retries

* On `nack` or processing error, set:
  * `attempts = attempts + 1`
  * `available_at = now + base * 2^attempts + jitter`
* If `attempts >= max_attempts`, drop the message (it will not be re-presented).

# Observability & ops

* Structured logs with spans: `tracing` + `tracing-subscriber`.
* Metrics: request counts/latency, DB time, gauges, **retries**. Expose `/metrics` (Prometheus).
* Traces: OpenTelemetry export; instrument DB calls and critical paths.
* Admin tools: `purge queue`, `peek`, `compact`.
* Rate-limits & quotas per queue (basic token bucket).

# Failure & recovery scenarios to handle

* Clock skew: base scheduling on **server time** only.
* Partial writes: wrap enqueue/ack/nack in transactions; keep them **short**.
* Large bursts: apply backpressure (429) when DB write queue grows or WAL grows beyond a threshold.

# Rust stack (battle-tested picks)

* HTTP: `axum` (or `actix-web`), middleware via `tower`.
* JSON: `serde` + `serde_json`.
* DB: `sqlx` (async, compile-time checked) **or** `rusqlite` (sync; fine if you isolate DB work on a blocking thread pool).
* Migrations: `sqlx::migrate!()` or `refinery`.
* IDs & tokens: `uuid`, `rand`, `base64`.
* Errors: `thiserror` + `anyhow` at the boundary.
* Config: `figment` or `config`.
* Auth: `jsonwebtoken` (if you want JWT per queue) or simple opaque API keys.
* Tests: spawn the server with a temp DB (`tempfile`), run integration tests over HTTP.


# Security & multi-tenancy

* **API keys per queue** (store hashed in DB). Use `Authorization: Bearer <key>`.
* Optional **HMAC** of body using queue secret to detect tampering.
* Per-queue **quotas** and **rate limits**; reject rogue producers.
* Validate JSON strictly; drop massive/recursive payloads; set request body limits.
* CORS defaults to **off** unless you need browser producers.

---

## Flamegraphs (Performance)

- Prereqs: `cargo install flamegraph`.
  - Linux: requires `perf` (install via your distro).
  - macOS: uses `dtrace` and typically requires `sudo`.
- A concurrent stress test exists at `tests/stress_tests.rs` (ignored by default).
- Script to generate a flamegraph of the stress test:
  - `scripts/flame-stress.sh [--dev-profile] [mode] [concurrency] [total] [outdir]`
  - Modes: `enqueue` (default), `drain`, `mixed`.
  - Examples:
    - `scripts/flame-stress.sh` (defaults: 32, 2000, `flamegraphs/`)
    - `scripts/flame-stress.sh 64 10000`
  - The script sets `RUSTFLAGS="-C debuginfo=2 -C force-frame-pointers=yes -C lto=no -C codegen-units=1"` for better symbolization; on Linux it also passes `--no-inline`. Output path: `flamegraphs/flame-stress-<mode>-<conc>-<total>-<timestamp>.svg`.
  - If symbols still look incomplete:
    - Use debug profile: `cargo flamegraph --dev --test stress_tests -- --ignored <name>`
    - Ensure binaries aren’t stripped; keep frame pointers.
  - To force dev profile via script, pass `--dev-profile` as the first argument:
    - `scripts/flame-stress.sh --dev-profile mixed 32 2000`

---

## Quick Start

- Build the project:
  - `cargo build` (or `cargo build --release`)
- Run the server locally (listens on 127.0.0.1):
  - `cargo run -- serve --port 8888`
- Create a queue via CLI:
  - `cargo run -- queue add --name demo --max-attempts 5`
- Enqueue a message via CLI:
  - `cargo run -- message enqueue --queue demo --payload '{"hello":"world"}'`
- Poll and ack via CLI:
  - `cargo run -- message poll --queue demo --batch 1 --visibility-ms 30000`
  - `cargo run -- message ack --ids <id1,id2>`

## CLI Usage (Implemented)

- Server
  - `sqew serve --port 8888`
- Queues
  - `sqew queue list`
  - `sqew queue add --name <name> --max-attempts <n>`
  - `sqew queue show --name <name>`
  - `sqew queue purge --name <name>`
  - `sqew queue peek --name <name> --limit <n>`
  - `sqew queue remove --name <name>`
  - `sqew queue compact --name <name>` (VACUUM)
- Messages
  - `sqew message enqueue --queue <name> --payload '<json>' [--delay-ms <ms>]`
  - `sqew message enqueue --queue <name> --file <ndjson-or-json-array> [--delay-ms <ms>]`
  - `sqew message poll --queue <name> --batch <n> --visibility-ms <ms>`
  - `sqew message ack --ids <id1,id2,...>`
  - `sqew message nack --ids <id1,id2,...> --delay-ms <ms>`
  - `sqew message remove --id <id>`
  - `sqew message peek --queue <name> --limit <n>`
  - `sqew message peek-id --id <id>`

Notes
- Delivery is at-least-once. Duplicates can occur under concurrency; always ack after successful processing.
- Visibility timeout controls lease duration for polled messages; unacked leases become visible again after the timeout.

## HTTP API (Implemented)

- Health
  - `GET /health` → `200 ok`
- Queues
  - `GET /queues` → `200` JSON array of queues
  - `POST /queues` body `{ "name": "q", "max_attempts": 5 }` → `201` queue
  - `GET /queues/{name}` → `200` queue or `404`
  - `DELETE /queues/{name}` → `204` or `404`
  - `GET /queues/{name}/stats` → `200` `{ "ready": <i64> }`
- Messages
  - `GET /queues/{name}/messages?limit=N` → `200` list (peek; no leasing)
  - `POST /queues/{name}/messages` body `{ "payload": <json>, "delay_ms": 0 }` → `201` created message
  - `DELETE /queues/{name}/messages` → `200` `{ "deleted": <u64> }`

Examples (curl)
- Create a queue
  - `curl -s localhost:8888/queues -X POST -H 'content-type: application/json' -d '{"name":"demo","max_attempts":5}'`
- Enqueue a message
  - `curl -s localhost:8888/queues/demo/messages -X POST -H 'content-type: application/json' -d '{"payload":{"k":"v"}}'`
- Peek up to 10 messages
  - `curl -s 'localhost:8888/queues/demo/messages?limit=10'`
- Queue stats
  - `curl -s localhost:8888/queues/demo/stats`
- Purge
  - `curl -s localhost:8888/queues/demo/messages -X DELETE`

## Storage & Configuration

- Database: SQLite file `sqew.db` at the project root by default (git-ignored).
- The service configures SQLite for concurrency: WAL mode, busy_timeout, synchronous=NORMAL.
- CLI and tests create the DB if missing and apply the embedded schema.
- Custom DB path (library): use `queue::Config { db_path, force_recreate }` with `queue::init_pool(&cfg)`.

## Development

- Build: `cargo build` (release: `cargo build --release`)
- Lint: `cargo clippy --all-targets`
- Format: `cargo fmt --all`
- Run server: `cargo run -- serve --port 8888`
- Bacon (optional): `bacon` then press `c` (clippy), `t` (test), `r` (run)

## Testing & Stress

- Unit/integration tests: `cargo test`
- Stress tests (in-process HTTP), configurable via env vars:
  - `SQEW_STRESS_TOTAL`, `SQEW_STRESS_CONCURRENCY`, `SQEW_STRESS_PRODUCERS`, `SQEW_STRESS_CONSUMERS`, `SQEW_STRESS_BATCH`, `SQEW_STRESS_VIS_MS`
- Run a single stress test:
  - Enqueue-only: `cargo test --test stress_tests -- --exact concurrent_enqueue_no_loss --nocapture`
  - Enqueue+drain: `cargo test --test stress_tests -- --exact concurrent_enqueue_and_drain_no_loss --nocapture`
  - Mixed produce/consume: `cargo test --test stress_tests -- --exact concurrent_mixed_produce_consume_counts_ok --nocapture`

## Docker

- Build image:
  - `docker build -t sqew:latest .`
- Run server (binds to 0.0.0.0:8888 in container):
  - `docker run --rm -p 8888:8888 -v $(pwd)/data:/data -e SQEW_BIND=0.0.0.0 sqew:latest`
- Data location:
  - The container works from `/data` (default DB path: `/data/sqew.db`). Mount a host directory to persist data.
- Health check:
  - `curl -s localhost:8888/health`
- Create a queue:
  - `curl -s localhost:8888/queues -X POST -H 'content-type: application/json' -d '{"name":"demo","max_attempts":5}'`
