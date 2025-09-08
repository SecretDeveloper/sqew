# Sqew Project Plan

## 1. Project Initialization
- Create new Rust project (`cargo new sqew --bin`)
- Set up directory structure for CLI, service, models, migrations

## 2. Dependencies
- Add crates: axum, tokio, sqlx (sqlite), serde, uuid, tracing, anyhow, thiserror, etc.

## 3. Database Schema & Migrations
- Write initial SQLite schema and migration scripts
- Add migration tooling (sqlx::migrate!())

## 4. Core Models
- Define Rust structs for queues and messages
- Implement serialization/deserialization with serde

## 5. Database Layer
- Implement async DB access using sqlx
- Add helper functions for queue/message operations

## 6. HTTP API
- Set up Axum server and define routes for all endpoints
- Implement handlers for each route, including batch and DLQ

## 7. CLI Tool
- Implement CLI commands using clap or similar
- Ensure CLI covers all API features

## 8. Delivery Semantics
- Implement at-least-once delivery, visibility timeouts, lease tokens, idempotency key logic

## 9. Security
- Add per-queue API key authentication
- Implement request size limits and strict JSON validation
- Optional: HMAC signing

## 10. Observability
- Add structured logging and tracing
- Expose metrics in Prometheus format
- Implement health/readiness endpoints

## 11. Testing
- Write unit and integration tests (ephemeral DB)
- Test all core flows

## 12. Documentation
- Document API endpoints (OpenAPI/Swagger)
- Document CLI usage/configuration

## 13. Operational Features
- Implement periodic cleanup (expired messages, DB maintenance)
- Add admin/debug tools (purge, requeue, peek, compact)

## 14. Packaging & Deployment
- Build as a single binary
- Document deployment steps and SQLite file management
