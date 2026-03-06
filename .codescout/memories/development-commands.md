# Development Commands

See CLAUDE.md § Build & Run for primary commands. Additions:

## Quick Commands
- `cargo check` — fast type-check, no linking (preferred during development)
- `cargo build --release` — both binaries with LTO+strip (~6-7MB each)
- `cargo build --release --bin researcher` or `--bin researcher-mcp` — single binary

## Before Completing Work
1. `cargo check` — ensure no type errors
2. `cargo build --release` — verify it links and both binaries produce

## Notes
- No test suite currently — verify by running the binary manually
- Release profile uses `lto=true, codegen-units=1, strip=true` — slow to compile, small output
- Docker stack in CLAUDE.md § Docker Stack; `.env.example` has all needed vars
