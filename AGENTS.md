# Repository Guidelines

## Project Structure & Module Organization
- `src/main.rs`: entrypoint; wires CLI to runtime.
- `src/cli.rs`: CLI (`sqew`) commands and parsing (serve/queue/message).
- `src/server.rs`: Axum HTTP server and routes.
- `src/queue.rs`: service layer over DB (queues, stats, purge, peek).
- `src/db/`: SQLx helpers, schema bootstrap, counters, VACUUM.
- `src/models/`: shared structs (`Queue`, `Message`).
- Top-level: `Cargo.toml`, `rustfmt.toml`, `bacon.toml`, `README.md`, `sqew.db` (SQLite; git-ignored).

## Build, Test, and Development Commands
- Build: `cargo build` (release: `cargo build --release`).
- Run server: `cargo run -- serve --port 8888`.
- CLI examples:
  - `cargo run -- queue add --name demo --max-attempts 5`
  - `cargo run -- queue list`
  - `cargo run -- queue show --name demo`
- Lint: `cargo clippy --all-targets`.
- Format: `cargo fmt --all`.
- Bacon (optional dev loop): `bacon` then press `c` (clippy), `t` (test), `r` (run).

## Coding Style & Naming Conventions
- Rust 2024 edition. Format with `rustfmt.toml` (4-space indent, 80 cols, grouped imports).
- Naming: modules `snake_case`; types/enums `CamelCase`; functions/vars `snake_case`; constants `SCREAMING_SNAKE_CASE`.
- Errors: `anyhow` at boundaries; library errors via `thiserror` where appropriate.
- Logging: use `tracing`; initialize in `server` and for CLIs that perform work.

## Testing Guidelines
- Framework: `cargo test`. Prefer fast unit tests co-located in modules; integration tests in `tests/` (files `*_test.rs`).
- Async: use `#[tokio::test]` for async tests.
- DB: use a temp SQLite file (via `tempfile`) and connect with `SqlitePool::connect(format!("sqlite://{}", path))`; initialize by executing the embedded schema or calling `db::create_db_if_needed()` against the temp path.
- Add tests for new routes and service functions (e.g., create queue → list → stats flow).

## Commit & Pull Request Guidelines
- Commits: imperative, present tense, concise subject (≤72 chars). Example: `queue: add stats endpoint and tests`.
- PRs: clear description (what/why), linked issues, steps to verify (sample `curl` or CLI), and updated docs when behavior changes. Include tests for features/fixes.

## Security & Configuration Tips
- SQLite file lives at project root as `sqew.db` (git-ignored). Don’t commit real data.
- Prefer running the server locally on `127.0.0.1` unless testing remote access.
