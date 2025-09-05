# Agents – Lightweight Message Queue in Rust + SQLite

## Sqew

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
  - Configure visibility timeout and retry limit.

- **Message Lifecycle**
  - Enqueue JSON payloads with optional delay, priority, TTL, and idempotency key.
  - Poll/lease messages atomically (batch support).
  - Acknowledge (ack) processed messages → delete permanently.
  - Negative acknowledge (nack) → increment attempts and reschedule; messages exceeding the retry limit will be dropped.
  - Expired messages (TTL) automatically dropped.

- **APIs**
  - REST/HTTP endpoints with JSON.
  - Admin/debug endpoints (peek, stats, purge, requeue).

- **Delivery Semantics**
  - At-least-once delivery.
  - Visibility timeout / lease system to requeue unacked messages.
  - Idempotency key support for enqueue (deduplication).
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

* Delivery guarantees: keep it **at-least-once**; require **idempotent consumers** (idempotency keys) if you need “effectively once”.  Idempotency is guaranteed per-queue, not globally.
* Visibility/lease: messages are invisible after a “pop” until **lease\_expires\_at**; if not acked before then, they reappear.
* Ordering: SQLite can’t guarantee strict FIFO under concurrency. Use `(priority DESC, available_at, id)` ordering and document it as **best-effort ordering**.
* Deduplication: optional **idempotency\_key** on enqueue with a unique index (per queue) → return existing message if duplicate.
* Retries: track **attempts** with **max\_attempts**, exponential backoff to **available\_at** on `nack`.
* Messages exceeding `max_attempts` will be dropped and not re-presented to consumers.
* TTL: drop after **expires\_at** to avoid zombie messages.
* Size limits: enforce max payload size (e.g. 512 KB JSON) to keep DB healthy.

---

## Specification

### CLI Usage

```bash
 sqew serve # Will host a new sqew service at port 8888 by default
 sqew --help # Displays help information
 sqew --version # Shows version information
 sqew queue list # List available queues
 sqew queue add --name {name} --max-attempts <max_attempts:5> --visibility-ms <30000> # Adds a new queue
 sqew queue rm --name {name} # Delete a queue
 sqew queue show --name {name} # Show information and stats about the queue
 sqew queue purge --name {name} # Purge messages from queue
 sqew queue peek --name {name} --limit <n> # Peek N items from queue
 sqew queue compact --name {name}
 sqew message remove --queue {name} --id <id> # Remove messages from queue by id
 sqew message peek --queue {name} --id <id> # Peek message from queue by id
 sqew message get --queue {name} --batch <n> # Get message(s), default is 1>
 sqew message enqueue --queue {name} --file <json_file> --payload <json> --idempotency-key <key> # enqueue messages from a newline-delimited JSON file
 sqew message poll --queue {name} --batch <n> # poll/lease up to n messages
 sqew message ack --queue {name} --ids <id1,id2,...> # acknowledge multiple messages
 sqew message nack --queue {name} --ids <id1,id2,...> [--delay-ms <ms>] # nack multiple messages
 sqew message extend-lease --queue {name} --id <id> --ms <duration> # Extend lease of message
 sqew stats --queue {name} #(show queue stats)
 sqew health # (check service health)
 sqew metrics # (show metrics in Prometheus format)
```


# HTTP/JSON API sketch

* `POST /queues` → create queue `{name, dlq, max_attempts, visibility_ms}`
* `POST /queues/{q}/messages` → enqueue `{payload, priority?, delay_ms?, idempotency_key?}` → `{message_id}`.  This can be batched with multiple messages.
* `POST /queues/{q}/poll?batch=n` → long-poll up to `n` ready messages, **atomically lease** them for `visibility_ms`, return `{messages: [{id, payload, lease_expires_at, token}]}`

  * Use long-poll timeout (e.g. 20–30s) to reduce spin.
* `POST /queues/{q}/ack` → `{id, token}`; delete message within a transaction.
* `POST /queues/{q}/nack` → `{id, token, delay_ms?}`; bump `attempts`, reschedule `available_at` using backoff.
* `GET /queues/{q}/stats` → counts (ready, leased, dlq, oldest age, etc.)
* `GET /healthz` / `GET /readyz`
* Optional: `DELETE /queues/{q}/messages/{id}` (admin), `GET /queues/{q}/peek?limit=...` (debug)

# Leasing atomically (race-free pattern)

* Generate a **lease token** (UUID) and run one `UPDATE … WHERE … RETURNING` to both claim and read rows.
* Always include `token` (server-generated secret) in `ack/nack` to **fence** other workers that might see the same message after lease expiry.

# Backoff & retries

* On `nack` or processing error, set:
  * `attempts = attempts + 1`
  * `available_at = now + base * 2^attempts + jitter`
* If `attempts >= max_attempts`, drop the message (it will not be re-presented).

# Idempotency story

* **Producers**: accept `Idempotency-Key` header → map to `idempotency_key`; if unique constraint hit, return the original `message_id`.
* **Consumers**: require clients to send a **consumer-side idempotency key** to their own systems; recommend storing processed message IDs to drop duplicates.

# Observability & ops

* Structured logs with spans: `tracing` + `tracing-subscriber`.
* Metrics: request counts/latency, DB time, **ready vs leased** gauges, **retries**. Expose `/metrics` (Prometheus).
* Traces: OpenTelemetry export; instrument DB calls and critical paths.
* Admin tools: `purge queue`, `peek`, `compact`.
* Rate-limits & quotas per queue (basic token bucket).

# Failure & recovery scenarios to handle

* Crashes during lease: rely on **lease timeout** to re-surface messages.
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

