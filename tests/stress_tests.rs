use std::time::Duration;

use serde_json::json;
use sqew::queue::{self, Config};
use sqew::server::app_router;
use tower::ServiceExt; // for `oneshot`
use axum::{body::{Body, to_bytes}, http::{Request, StatusCode}};
use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};
use tokio::sync::Mutex;
use std::collections::HashSet;

// Helper to build a test Config pointing to a temp DB
fn test_config(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = Config::default();
    cfg.db_path = tmp.path().join("stress.db");
    cfg.force_recreate = true;
    cfg
}

async fn enqueue_http_with_retry(
    app: axum::Router,
    qname: &str,
    body: serde_json::Value,
    max_retries: usize,
) -> anyhow::Result<()> {
    for attempt in 0..=max_retries {
        let req = Request::builder()
            .method("POST")
            .uri(format!("/queues/{}/messages", qname))
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&body)?))?;
        let resp = app.clone().oneshot(req).await?;
        let status = resp.status();
        if status == StatusCode::CREATED {
            return Ok(());
        }
        // Extract body for diagnostics and retry on transient DB lock
        let bytes = to_bytes(resp.into_body(), 64 * 1024).await.unwrap_or_default();
        let text = String::from_utf8_lossy(&bytes);
        let is_transient = status.is_server_error() && text.contains("locked");
        if attempt < max_retries && is_transient {
            tokio::time::sleep(Duration::from_millis(5)).await;
            continue;
        }
        anyhow::bail!("enqueue failed: {} {}", status, text);
    }
    Ok(())
}

async fn poll_with_retry(
    pool: &sqlx::SqlitePool,
    qname: &str,
    batch: i64,
    vis_ms: i64,
    max_retries: usize,
) -> anyhow::Result<Vec<sqew::models::Message>> {
    for attempt in 0..=max_retries {
        match queue::poll_messages(pool, qname, batch, vis_ms).await {
            Ok(v) => return Ok(v),
            Err(e) => {
                let s = format!("{e:#}");
                if attempt < max_retries && s.contains("database is locked") {
                    let backoff = 5 * (attempt as u64 + 1);
                    tokio::time::sleep(Duration::from_millis(backoff.min(50))).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
    Ok(Vec::new())
}

async fn ack_with_retry(pool: &sqlx::SqlitePool, ids: &[i64], max_retries: usize) -> anyhow::Result<u64> {
    for attempt in 0..=max_retries {
        match queue::ack_messages(pool, ids).await {
            Ok(n) => return Ok(n),
            Err(e) => {
                let s = format!("{e:#}");
                if attempt < max_retries && s.contains("database is locked") {
                    let backoff = 5 * (attempt as u64 + 1);
                    tokio::time::sleep(Duration::from_millis(backoff.min(50))).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
    Ok(0)
}

#[tokio::test]
//#[ignore]
async fn concurrent_enqueue_no_loss() -> anyhow::Result<()> {
    // Parameters via env for flexibility
    let concurrency: usize = std::env::var("SQEW_STRESS_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32);
    let total: usize = std::env::var("SQEW_STRESS_TOTAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);

    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = queue::init_pool(&cfg).await?;

    // Create queue before starting the server to avoid races
    let qname = "stress";
    let _q = queue::create_queue(&pool, qname, 5).await?;

    // Build the in-process app router (no sockets)
    let app = app_router(pool.clone());

    // Partition total across workers
    let per = total / concurrency;
    let extra = total % concurrency;

    let mut tasks = Vec::with_capacity(concurrency);
    for w in 0..concurrency {
        let app = app.clone();
        let count = per + if w < extra { 1 } else { 0 };
        tasks.push(tokio::spawn(async move {
            for i in 0..count {
                let body = json!({
                    "payload": {"worker": w, "seq": i},
                    "delay_ms": 0
                });
                enqueue_http_with_retry(app.clone(), qname, body, 50).await?;
            }
            anyhow::Ok(())
        }));
    }

    // Await all workers
    for t in tasks {
        t.await??;
    }

    // Poll DB until ready count reaches expected total or timeout
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as i64;
        let q = queue::show_queue(&pool, qname).await?;
        let ready = sqew::db::count_ready_messages(&pool, q.id, now).await?;
        if ready as usize == total { break; }
        if std::time::Instant::now() > deadline {
            anyhow::bail!("Timeout waiting for ready messages: got {} expected {}", ready, total);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // Additionally, verify raw total via DB to be safe
    let q = queue::show_queue(&pool, qname).await?;
    let queued = sqew::db::count_queued_messages_by_queue(&pool, q.id).await?;
    assert_eq!(queued as usize, total);

    Ok(())
}

#[tokio::test]
async fn concurrent_enqueue_and_drain_no_loss() -> anyhow::Result<()> {
    // Parameters
    let concurrency: usize = std::env::var("SQEW_STRESS_CONCURRENCY")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32);
    let total: usize = std::env::var("SQEW_STRESS_TOTAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let consumer_workers: usize = std::env::var("SQEW_STRESS_CONSUMERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let consumer_batch: i64 = std::env::var("SQEW_STRESS_BATCH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32);
    let visibility_ms: i64 = std::env::var("SQEW_STRESS_VIS_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000);

    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = queue::init_pool(&cfg).await?;

    // Create queue and app
    let qname = "stress";
    let _q = queue::create_queue(&pool, qname, 5).await?;
    let app = app_router(pool.clone());

    // Enqueue all messages over HTTP
    let per = total / concurrency;
    let extra = total % concurrency;
    let mut tasks = Vec::with_capacity(concurrency);
    for w in 0..concurrency {
        let app = app.clone();
        let count = per + if w < extra { 1 } else { 0 };
        tasks.push(tokio::spawn(async move {
            for i in 0..count {
                let body = json!({
                    "payload": {"worker": w, "seq": i},
                    "delay_ms": 0
                });
                let req = Request::builder()
                    .method("POST")
                    .uri(format!("/queues/{}/messages", qname))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body)?))?;
                let resp = app.clone().oneshot(req).await?;
                if resp.status() != StatusCode::CREATED { anyhow::bail!("enqueue failed: {}", resp.status()); }
            }
            anyhow::Ok(())
        }));
    }
    for t in tasks { t.await??; }

    // Consumers: drain the queue concurrently, acking everything
    let consumed = Arc::new(AtomicUsize::new(0));
    let seen = Arc::new(Mutex::new(HashSet::<i64>::new()));

    let mut consumers = Vec::with_capacity(consumer_workers);
    for _ in 0..consumer_workers {
        let pool = pool.clone();
        let consumed = consumed.clone();
        let seen = seen.clone();
        let qname = qname.to_string();
        consumers.push(tokio::spawn(async move {
            loop {
                let msgs = poll_with_retry(&pool, &qname, consumer_batch, visibility_ms, 500).await?;
                if msgs.is_empty() {
                    // Check if all work is done by looking at consumed counter and ready count
                    let q = queue::show_queue(&pool, &qname).await?;
                    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64;
                    let ready = sqew::db::count_ready_messages(&pool, q.id, now).await?;
                    if consumed.load(Ordering::Relaxed) >= total && ready == 0 { break; }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                let ids: Vec<i64> = msgs.iter().map(|m| m.id).collect();
                // Track uniqueness
                {
                    let mut set = seen.lock().await;
                    for id in &ids {
                        if !set.insert(*id) { anyhow::bail!("duplicate delivery detected for id={}", id); }
                    }
                }
                // Ack the polled messages
                let n = ack_with_retry(&pool, &ids, 200).await? as usize;
                consumed.fetch_add(n, Ordering::Relaxed);
            }
            anyhow::Ok(())
        }));
    }

    for c in consumers { c.await??; }

    // Final assertions: all consumed, none left in queue
    let q = queue::show_queue(&pool, qname).await?;
    let total_consumed = consumed.load(Ordering::Relaxed);
    assert_eq!(total_consumed, total, "consumed != total");
    let remaining = sqew::db::count_queued_messages_by_queue(&pool, q.id).await?;
    assert_eq!(remaining, 0, "remaining queued messages should be 0");
    Ok(())
}

#[tokio::test]
async fn concurrent_mixed_produce_consume_counts_ok() -> anyhow::Result<()> {
    // Parameters
    let producers: usize = std::env::var("SQEW_STRESS_PRODUCERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| std::env::var("SQEW_STRESS_CONCURRENCY").ok().and_then(|v| v.parse().ok()).unwrap_or(32));
    let consumers_n: usize = std::env::var("SQEW_STRESS_CONSUMERS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let total: usize = std::env::var("SQEW_STRESS_TOTAL")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2000);
    let consumer_batch: i64 = std::env::var("SQEW_STRESS_BATCH")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(32);
    let visibility_ms: i64 = std::env::var("SQEW_STRESS_VIS_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(60_000);

    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = queue::init_pool(&cfg).await?;
    let qname = "stress";
    let _q = queue::create_queue(&pool, qname, 5).await?;
    let app = app_router(pool.clone());

    // Shared counters and flags
    let produced = Arc::new(AtomicUsize::new(0));
    let consumed = Arc::new(AtomicUsize::new(0));
    let producers_done = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(HashSet::<i64>::new()));

    // Spawn consumers first so reads and writes intermix
    let mut consumer_tasks = Vec::with_capacity(consumers_n);
    for _ in 0..consumers_n {
        let pool = pool.clone();
        let consumed = consumed.clone();
        let produced = produced.clone();
        let producers_done = producers_done.clone();
        let seen = seen.clone();
        let qname = qname.to_string();
        consumer_tasks.push(tokio::spawn(async move {
            loop {
                let msgs = queue::poll_messages(&pool, &qname, consumer_batch, visibility_ms).await?;
                if msgs.is_empty() {
                    // Exit if producers finished, everything produced was consumed, and nothing is ready
                    if producers_done.load(Ordering::Relaxed) {
                        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_millis() as i64;
                        let q = queue::show_queue(&pool, &qname).await?;
                        let ready = sqew::db::count_ready_messages(&pool, q.id, now).await?;
                        if consumed.load(Ordering::Relaxed) >= produced.load(Ordering::Relaxed) && ready == 0 {
                            break;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                    continue;
                }
                // Track seen IDs (duplicates are acceptable under at-least-once semantics)
                let ids: Vec<i64> = msgs.iter().map(|m| m.id).collect();
                { let mut set = seen.lock().await; for id in &ids { set.insert(*id); } }
                let acked = queue::ack_messages(&pool, &ids).await? as usize;
                let new_total = consumed.fetch_add(acked, Ordering::Relaxed) + acked;
                // Safety check: never consume more than produced so far
                let p = produced.load(Ordering::Relaxed);
                if new_total > p {
                    anyhow::bail!("consumed {} > produced {}", new_total, p);
                }
            }
            anyhow::Ok(())
        }));
    }

    // Spawn producers
    let per = total / producers;
    let extra = total % producers;
    let mut producer_tasks = Vec::with_capacity(producers);
    for w in 0..producers {
        let app = app.clone();
        let produced = produced.clone();
        let count = per + if w < extra { 1 } else { 0 };
        producer_tasks.push(tokio::spawn(async move {
            for i in 0..count {
                let body = json!({
                    "payload": {"worker": w, "seq": i},
                    "delay_ms": 0
                });
                enqueue_http_with_retry(app.clone(), qname, body, 50).await?;
                produced.fetch_add(1, Ordering::Relaxed);
            }
            anyhow::Ok(())
        }));
    }

    // Wait for producers to finish
    for t in producer_tasks { t.await??; }
    producers_done.store(true, Ordering::Relaxed);

    // Wait for consumers to drain all
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    for c in consumer_tasks { c.await??; }
    // Final checks
    let p = produced.load(Ordering::Relaxed);
    let c = consumed.load(Ordering::Relaxed);
    assert_eq!(p, total, "produced != total");
    assert_eq!(c, p, "consumed != produced");
    let q = queue::show_queue(&pool, qname).await?;
    let remaining = sqew::db::count_queued_messages_by_queue(&pool, q.id).await?;
    assert_eq!(remaining, 0, "remaining queued messages should be 0");
    // Sanity: no timeouts
    assert!(std::time::Instant::now() <= deadline, "mixed test exceeded deadline");
    Ok(())
}
