# Dual-Model Routing Design

**Date:** 2026-03-11
**Status:** Approved

## Overview

Split LLM inference across two GPUs: a fast small model on the AMD RX 7800 XT for structured pipeline tasks, and the heavy model on the NVIDIA A5000 for final report generation. This reduces pipeline latency (planner, summarizer, judge no longer queue behind heavy-model slots) and frees A5000 capacity for other projects.

## Hardware

| GPU | VRAM | Role |
|-----|------|------|
| NVIDIA RTX A5000 | 24GB | Heavy model (Qwen3.5-27B Q4_K_M, ~16GB) — final report only |
| AMD RX 7800 XT (gfx1101, RDNA3) | 16GB | Fast model (Qwen3-4B Q4_K_M, ~2.5GB) — planner, summarizer, judge |

## Infrastructure

### New service: `llama-cpp-fast`

Added to `infra/docker-compose.yml` alongside existing `llama-cpp`:

| Setting | `llama-cpp` (heavy) | `llama-cpp-fast` (fast) |
|---------|---------------------|-------------------------|
| Image | `server-cuda` | `server-rocm` |
| GPU | A5000 (`device_ids: ["0"]`) | RX 7800 XT (`/dev/kfd`, `/dev/dri`) |
| Model | Qwen3.5-27B Q4_K_M (~16GB) | Qwen3-4B Q4_K_M (~2.5GB) |
| Port | 30080 | 30081 |
| Context | 16384 | 8192 |
| Parallel slots | 2 | 4 |

Both services share the same model volume mount (`~/.lmstudio/models:/models:ro`) and run on `ai-infra-net`.

### Env vars (infra/.env)

```
LLAMA_FAST_MODEL=bartowski/Qwen3-4B-GGUF/Qwen3-4B-Q4_K_M.gguf
LLAMA_FAST_CTX_SIZE=8192
LLAMA_FAST_PARALLEL=4
```

## Rust Code Changes

### Config

New fields in `Config` (`src/config.rs`) and `config_from_env()` (`src/mcp_server.rs`):

| Field | Env var | Default |
|-------|---------|---------|
| `llm_fast_base_url` | `LLM_FAST_BASE_URL` | `""` (empty = use heavy backend) |
| `llm_fast_model` | `LLM_FAST_MODEL` | `Qwen3-4B-Q4_K_M` |
| `llm_fast_max_tokens` | `LLM_FAST_MAX_TOKENS` | `2048` |

`LLM_TEMPERATURE` and `STRIP_THINKING_TOKENS` are shared by both clients.

### LlmClient

Add `LlmClient::new_fast(cfg)` constructor that reads the `_FAST_` config fields. If `llm_fast_base_url` is empty, falls back to the heavy backend config — existing single-model setups work unchanged.

No structural changes to `LlmClient` itself — same `complete()` and `stream()` methods.

### Pipeline routing

`run()` in `src/researcher/pipeline.rs` creates two clients and passes the appropriate one:

| Stage | Client | Reason |
|-------|--------|--------|
| `planner::generate_queries()` | fast | Structured JSON output, simple task |
| `summarizer::summarize_all()` | fast | Structured JSON evaluation per source |
| `publisher::write_report()` | heavy | Free-form long-form report generation |

`crawler::crawl_all()` does not use LLM.

### Fallback behavior

When `LLM_FAST_BASE_URL` is empty (default), `new_fast()` returns a client pointing to the same heavy backend. This means:
- Existing single-model deployments need zero config changes
- Docker setups without `llama-cpp-fast` work as before
- The routing logic in the pipeline is always dual-client — only the underlying endpoint changes

## App Config

### docker-compose.yml (researcher)

Add environment variables:
```yaml
- LLM_FAST_BASE_URL=${LLM_FAST_BASE_URL:-http://llama-cpp-fast:8080/v1}
- LLM_FAST_MODEL=${LLM_FAST_MODEL:-Qwen3-4B-Q4_K_M}
- LLM_FAST_MAX_TOKENS=${LLM_FAST_MAX_TOKENS:-2048}
```

### MCP config

Both `~/.claude/.claude.json` and `~/.claude-sdd/.claude.json` need:
```json
"LLM_FAST_BASE_URL": "http://localhost:30081/v1",
"LLM_FAST_MODEL": "Qwen3-4B-Q4_K_M"
```

### .env.example

Add documented `LLM_FAST_*` variables alongside existing `LLM_*` section.

## Model Scaling Path

If Qwen3-4B proves too weak for planner/summarizer quality:
1. Qwen3-8B Q4_K_M (~5.5GB) — still leaves 10GB headroom
2. Qwen3-14B Q4_K_M (~9GB) — max useful size for 16GB card

No code changes needed — just swap `LLAMA_FAST_MODEL` in `infra/.env`.

## Constraints & Notes

- ROCm on RDNA3 consumer cards is less battle-tested than CUDA. If `server-rocm` image has issues with gfx1101, fallback is native llama.cpp binary with ROCm compiled from source.
- `/no_think` prefix stays on planner/summarizer system prompts — Qwen3-4B supports the same `/no_think` mechanism.
- `STRIP_THINKING_TOKENS` applies globally to both clients (same config field).
- The fast model's `max_tokens=2048` is sufficient for structured JSON responses.
