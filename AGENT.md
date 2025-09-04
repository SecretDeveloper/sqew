# Agents – Lightweight Message Queue in Rust + SQLite

## Sqew

Sqew is a lightweight message queue service and cli tool.

## Project Goals
- Build a **lightweight, embeddable message queue service** with HTTP + JSON endpoints.
- Focus on **simplicity, reliability, and portability**: no external dependencies beyond SQLite.
- Provide **at-least-once delivery** with configurable visibility timeouts and retries.
- Keep the system **observable** (metrics, logs, health checks) and **operationally safe** (DLQ, TTLs, cleanup).
- Serve as a **learning project** in Rust for building a production-style system with async HTTP, SQLite persistence, and clean modular architecture.

---

## Requirements

### Functional
- **Queue Management**
  - Create and manage named queues.
  - Configure visibility timeout, retry limit, DLQ reference.

- **Message Lifecycle**
  - Enqueue JSON payloads with optional delay, priority, TTL, and idempotency key.
  - Poll/lease messages atomically (batch support).
  - Acknowledge (ack) processed messages → delete permanently.
  - Negative acknowledge (nack) → increment attempts, reschedule, or route to DLQ.
  - Dead-letter queue for poison messages.
  - Expired messages (TTL) automatically dropped.

- **APIs**
  - REST/HTTP endpoints with JSON.
  - Optional long-polling for efficient consumers.
  - Admin/debug endpoints (peek, stats, purge, requeue).

- **Delivery Semantics**
  - At-least-once delivery.
  - Visibility timeout / lease system to requeue unacked messages.
  - Idempotency key support for enqueue (deduplication).

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
* Dead-letter: Created by default for each queue; send messages there when `attempts >= max_attempts`.
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
 sqew queue requeue-dlq --name {name} # Requeue DLQ items to Queue
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
 sqew dlq list --queue {name} #(list DLQ messages for a queue)
 sqew dlq purge --queue {name} #(purge DLQ messages)
 sqew dlq requeue --queue {name} #(requeue DLQ messages to main queue)
 sqew stats --queue {name} #(show queue stats)
 sqew health # (check service health)
 sqew metrics # (show metrics in Prometheus format)
```

# SQLite specifics (to avoid foot-guns)

* Turn on WAL & sane pragmas at startup:

  * `PRAGMA journal_mode=WAL;`
  * `PRAGMA synchronous=NORMAL;` (or `FULL` if you want maximum durability)
  * `PRAGMA busy_timeout=5000;`
  * `PRAGMA foreign_keys=ON;`
  * Consider `PRAGMA mmap_size`/`cache_size` for perf.
* Concurrency: SQLite has one writer; keep writes short, do **batch writes**, and reuse connections (pool).
* Indices matter: you’ll query by `(queue_id, available_at, leased_by, lease_expires_at)`.
* Maintenance: periodic `VACUUM` (rare), `ANALYZE`, and a cleanup job (`DELETE` old rows) to keep file small.
* Backup/restore: snapshot the `.db` + `.db-wal` atomically or use the SQLite Online Backup API.
* Encryption: use OS-level disk encryption, or SQLCipher if you need DB-level encryption.

# Initial Data model (SQL)

```sql
CREATE TABLE queue (
  id            INTEGER PRIMARY KEY,
  name          TEXT UNIQUE NOT NULL,
  dlq_id        INTEGER REFERENCES queue(id) ON DELETE SET NULL,
  max_attempts  INTEGER NOT NULL DEFAULT 5,
  visibility_ms INTEGER NOT NULL DEFAULT 30000
);

CREATE TABLE message (
  id               INTEGER PRIMARY KEY,
  queue_id         INTEGER NOT NULL REFERENCES queue(id) ON DELETE CASCADE,
  payload_json     TEXT NOT NULL,
  priority         INTEGER NOT NULL DEFAULT 0,
  idempotency_key  TEXT,
  attempts         INTEGER NOT NULL DEFAULT 0,
  available_at     INTEGER NOT NULL,         -- epoch ms
  lease_token      TEXT,                     -- lease token
  lease_expires_at INTEGER,                  -- epoch ms, NULL if not leased
  leased_by        TEXT,                     -- node id
  created_at       INTEGER NOT NULL,
  expires_at       INTEGER,                  -- optional TTL
  UNIQUE(queue_id, idempotency_key)
);

-- Helpful indexes
CREATE INDEX ix_msg_ready ON message(queue_id, lease_expires_at, available_at, priority DESC);
CREATE INDEX ix_msg_visible ON message(queue_id, available_at) WHERE lease_expires_at IS NULL;
CREATE INDEX ix_msg_leased ON message(queue_id, lease_expires_at) WHERE lease_expires_at IS NOT NULL;
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
* If you target older SQLite, do it as: `SELECT id … LIMIT n FOR UPDATE` is not available; instead use a single `UPDATE` with a CTE.
  Example:

```sql
WITH c AS (
  SELECT id FROM message
  WHERE queue_id=?1
    AND (lease_expires_at IS NULL OR lease_expires_at <= ?now)
    AND available_at <= ?now
  ORDER BY priority DESC, available_at, id
  LIMIT ?n
)
UPDATE message
   SET leased_by=?node, lease_expires_at=?now_plus_vis
 WHERE id IN (SELECT id FROM c)
RETURNING id, payload_json, lease_expires_at;
```

* Always include `token` (server-generated secret) in `ack/nack` to **fence** other workers that might see the same message after lease expiry.

# Backoff & retries

* On `nack` or processing error, set:

  * `attempts = attempts + 1`
  * `available_at = now + base * 2^attempts + jitter`
* If `attempts >= max_attempts`, move to DLQ (`INSERT INTO message(queue_id=dlq_id, …)`) and delete from source.

# Idempotency story

* **Producers**: accept `Idempotency-Key` header → map to `idempotency_key`; if unique constraint hit, return the original `message_id`.
* **Consumers**: require clients to send a **consumer-side idempotency key** to their own systems; recommend storing processed message IDs to drop duplicates.

# Observability & ops

* Structured logs with spans: `tracing` + `tracing-subscriber`.
* Metrics: request counts/latency, DB time, **ready vs leased** gauges, **retries**, **DLQ inflow**. Expose `/metrics` (Prometheus).
* Traces: OpenTelemetry export; instrument DB calls and critical paths.
* Admin tools: `requeue DLQ`, `purge queue`, `peek`, `compact`.
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

# Example: enqueue + poll + ack (sketch)

```rust
// Cargo: axum, tokio, sqlx (features = ["sqlite", "runtime-tokio-rustls", "macros"]), serde, uuid, tracing

#[derive(serde::Deserialize)]
struct Enqueue { payload: serde_json::Value, priority: Option<i32>, delay_ms: Option<i64>, idempotency_key: Option<String> }

async fn enqueue(
    State(db): State<sqlx::SqlitePool>,
    Path(qname): Path<String>,
    Json(b): Json<Enqueue>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    let now = chrono::Utc::now().timestamp_millis();
    let delay = b.delay_ms.unwrap_or(0);
    let idk = b.idempotency_key.clone();
    let mut tx = db.begin().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let q: (i64,) = sqlx::query_as("SELECT id FROM queue WHERE name=?")
        .bind(&qname).fetch_one(&mut *tx).await.map_err(|_| StatusCode::NOT_FOUND)?;
    // Try insert
    let res = sqlx::query!(
        r#"INSERT INTO message(queue_id,payload_json,priority,available_at,created_at,idempotency_key)
           VALUES (?1, ?2, COALESCE(?3,0), ?4, ?4, ?5)
        "#,
        q.0,
        b.payload.to_string(),
        b.priority,
        now + delay,
        idk
    ).execute(&mut *tx).await;
    let msg_id = match res {
        Ok(r) => r.last_insert_rowid(),
        Err(e) if e.to_string().contains("UNIQUE") => {
            sqlx::query_scalar::<_, i64>("SELECT id FROM message WHERE queue_id=? AND idempotency_key=?")
                .bind(q.0).bind(idk.unwrap()).fetch_one(&mut *tx).await.unwrap()
        },
        Err(_) => return Err(StatusCode::INTERNAL_SERVER_ERROR)
    };
    tx.commit().await.unwrap();
    Ok(Json(serde_json::json!({ "id": msg_id })))
}

#[derive(serde::Deserialize)]
struct Poll { batch: Option<i64> }

async fn poll(State(db): State<sqlx::SqlitePool>, Path(qname): Path<String>, Query(p): Query<Poll>) -> Result<Json<serde_json::Value>, StatusCode> {
    let now = chrono::Utc::now().timestamp_millis();
    let n = p.batch.unwrap_or(1).clamp(1, 256);
    let node = uuid::Uuid::new_v4().to_string();
    // Lease ready messages atomically
    let rows = sqlx::query!(
        r#"
WITH c AS (
  SELECT m.id
  FROM message m
  JOIN queue q ON q.id = m.queue_id
  WHERE q.name = ?1
    AND (m.lease_expires_at IS NULL OR m.lease_expires_at <= ?2)
    AND m.available_at <= ?2
  ORDER BY m.priority DESC, m.available_at, m.id
  LIMIT ?3
)
UPDATE message
   SET leased_by = ?4,
       lease_expires_at = ?2 + (SELECT visibility_ms FROM queue WHERE name=?1)
 WHERE id IN (SELECT id FROM c)
RETURNING id, payload_json, lease_expires_at
"#, qname, now, n, node
    ).fetch_all(&db).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let token = uuid::Uuid::new_v4().to_string();
    let out: Vec<_> = rows.into_iter().map(|r| serde_json::json!({
        "id": r.id, "payload": serde_json::from_str::<serde_json::Value>(&r.payload_json.unwrap()).unwrap(),
        "lease_expires_at": r.lease_expires_at, "token": token
    })).collect();
    Ok(Json(serde_json::json!({ "messages": out })))
}
```

*(Ack/Nack are straightforward: verify token/fencing if you store it per-lease; `DELETE FROM message WHERE id=? AND leased_by=?` for ack; for nack, increment attempts, reschedule or DLQ.)*

# Security & multi-tenancy

* **API keys per queue** (store hashed in DB). Use `Authorization: Bearer <key>`.
* Optional **HMAC** of body using queue secret to detect tampering.
* Per-queue **quotas** and **rate limits**; reject rogue producers.
* Validate JSON strictly; drop massive/recursive payloads; set request body limits.
* CORS defaults to **off** unless you need browser producers.

# Performance tips

* Batch operations: allow `enqueue` and `ack` arrays.
* Keep leases short and extendable (heartbeat endpoint) if processing can exceed `visibility_ms`.
* Use a small worker that **reaps expired leases** and moves poison messages to DLQ.
* Prefer a single writer task per process (channel → DB) if you hit writer contention.

# Nice extras you’ll thank yourself for

* OpenAPI/Swagger (`utoipa`) so clients can self-serve.
* CLI admin tool (`sqew queue requeue-dlq`, `sqew queue show`).
* SSE or WebSocket **notify** channel for consumers who prefer push.
* Export snapshots to newline-delimited JSON for debugging/replay.
* Canary queue & synthetic messages for SLOs.

If you want, I can turn this into a `cargo new` skeleton with `axum + sqlx + migrations`, ready to run with the schema, endpoints, and a tiny load test.
