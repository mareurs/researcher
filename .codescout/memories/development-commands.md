# Development Commands

See CLAUDE.md for the primary command reference. Additions below.

## Before Completing Work
1. `cargo check` — fast type-check (no link)
2. `cargo build --release` — full build (LTO + strip); confirms both binaries compile
3. Manual smoke test if changing pipeline logic (no automated tests exist)

## Gotchas for Build
- Release profile uses LTO + `codegen-units=1` — slow but produces small binary (~6-7MB)
- Both binaries share the same `src/` module tree — changes to shared modules affect both
- `profiles.toml` must exist at the **working directory** when running; missing = silent empty profiles

## Quick Local Test (no Docker)
```bash
LLM_BASE_URL=http://localhost:8080/v1 \
SEARXNG_URL=http://localhost:4000 \
RUST_LOG=info \
cargo run --bin researcher -- --query "test topic"
```

## MCP Binary Config (env-only, no CLI flags)
```bash
LLM_BASE_URL=... SEARXNG_URL=... cargo run --bin researcher-mcp
```
The MCP binary ignores all clap args — everything is env vars via `config_from_env()`.
