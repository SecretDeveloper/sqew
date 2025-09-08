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

