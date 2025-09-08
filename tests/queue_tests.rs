use std::path::PathBuf;

use serde_json::json;
use sqew::queue::{
    Config, ack_messages, compact, create_queue, delete_queue, enqueue_message,
    get_message_by_id, init_pool, list_queues, nack_messages, peek_queue,
    poll_messages, purge_queue, show_queue, stats,
};

fn test_config(tmp: &tempfile::TempDir) -> Config {
    let mut cfg = {
        let cwd =
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        Config { db_path: cwd.join("sqew.db"), force_recreate: false }
    };
    cfg.db_path = tmp.path().join("test.db");
    cfg.force_recreate = true;
    cfg
}

#[tokio::test]
async fn queue_create_list_show_delete() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = init_pool(&cfg).await?;

    // Initially empty
    assert!(list_queues(&pool).await?.is_empty());

    // Create
    let q = create_queue(&pool, "demo", 2).await?;
    assert_eq!(q.name, "demo");

    // List & show
    let all = list_queues(&pool).await?;
    assert_eq!(all.len(), 1);
    let got = show_queue(&pool, "demo").await?;
    assert_eq!(got.id, q.id);

    // Delete
    assert!(delete_queue(&pool, "demo").await?);
    assert!(list_queues(&pool).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn enqueue_peek_get_and_purge() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = init_pool(&cfg).await?;
    let _q = create_queue(&pool, "q1", 5).await?;

    // Enqueue two messages
    let m1 = enqueue_message(&pool, "q1", &json!({"n":1}), 0).await?;
    let m2 = enqueue_message(&pool, "q1", &json!({"n":2}), 0).await?;
    assert!(m1.id > 0 && m2.id > m1.id);

    // Peek should see both
    let msgs = peek_queue(&pool, "q1", 10).await?;
    assert_eq!(msgs.len(), 2);

    // Get by id
    let g = get_message_by_id(&pool, m1.id).await?;
    assert_eq!(g.id, m1.id);

    // Purge
    let purged = purge_queue(&pool, "q1").await?;
    assert_eq!(purged, 2);
    assert!(peek_queue(&pool, "q1", 10).await?.is_empty());
    Ok(())
}

#[tokio::test]
async fn poll_and_ack() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = init_pool(&cfg).await?;
    let _q = create_queue(&pool, "q2", 5).await?;

    let m = enqueue_message(&pool, "q2", &json!({"task":"t"}), 0).await?;

    // Poll with visibility 100 ms
    let msgs = poll_messages(&pool, "q2", 1, 100).await?;
    assert_eq!(msgs.len(), 1);
    let leased = &msgs[0];
    assert_eq!(leased.id, m.id);
    assert!(leased.available_at > leased.created_at);

    // Ack deletes
    let n = ack_messages(&pool, &[leased.id]).await?;
    assert_eq!(n, 1);
    // Ensure not found
    assert!(get_message_by_id(&pool, leased.id).await.is_err());
    Ok(())
}

#[tokio::test]
async fn nack_and_drop_on_max_attempts() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = init_pool(&cfg).await?;
    let _q = create_queue(&pool, "q3", 2).await?; // max_attempts = 2

    let m = enqueue_message(&pool, "q3", &json!({"x":1}), 0).await?;

    // First nack -> requeue with attempts=1
    let (requeued, dropped) = nack_messages(&pool, &[m.id], 10).await?;
    assert_eq!((requeued, dropped), (1, 0));
    let after1 = get_message_by_id(&pool, m.id).await?;
    assert_eq!(after1.attempts, 1);

    // Second nack -> attempts becomes 2, equals max_attempts => drop
    let (requeued2, dropped2) = nack_messages(&pool, &[m.id], 10).await?;
    assert_eq!((requeued2, dropped2), (0, 1));
    assert!(get_message_by_id(&pool, m.id).await.is_err());
    Ok(())
}

#[tokio::test]
async fn stats_and_compact() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let cfg = test_config(&dir);
    let pool = init_pool(&cfg).await?;
    let _q = create_queue(&pool, "q4", 5).await?;
    let _ = enqueue_message(&pool, "q4", &json!({"n":1}), 0).await?;
    let _ = enqueue_message(&pool, "q4", &json!({"n":2}), 1000).await?;

    // Ready should be >= 1 (first message available now)
    let s = stats(&pool, "q4").await?;
    assert!(s.get("ready").and_then(|v| v.as_i64()).unwrap_or(0) >= 1);

    // Compact shouldn't error
    compact(&pool).await?;
    Ok(())
}
