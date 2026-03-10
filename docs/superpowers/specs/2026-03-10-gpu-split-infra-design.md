# GPU Split Infrastructure Design

**Date:** 2026-03-10
**Status:** Approved

## Overview

Split the single `docker-compose.yml` into two independent stacks:
- `infra/docker-compose.yml` — shared AI infrastructure (llama-cpp, TEI, SearXNG); always-on, reusable across projects
- `docker-compose.yml` — researcher Rust binary only; joins infra network as external

GPU load is split by workload:
- **NVIDIA RTX A5000 (24GB)** — llama-cpp only; freed from TEI to run a larger model
- **AMD RX 7800 XT (Navi 32)** — display only; no ML workload (neither TEI nor infinity have reliable consumer RDNA3 Docker support)
- **CPU** — TEI embed + rerank + SearXNG

## Directory Structure

```
researcher/
├── infra/
│   ├── docker-compose.yml    ← ai-infra stack
│   └── .env.example          ← infra tunables (model path, ctx size, COMPOSE_PROJECT_NAME)
├── docker-compose.yml        ← researcher only
├── .env.example              ← researcher tunables
└── Makefile                  ← convenience targets
```

## Network

- External Docker network: `ai-infra-net`
- Created and owned by `infra/docker-compose.yml`
- Declared `external: true` in root `docker-compose.yml`
- Other projects reuse by joining the same network

## Services

### infra/docker-compose.yml

| Service | Image | GPU | Port |
|---------|-------|-----|------|
| `llama-cpp` | `ghcr.io/ggml-org/llama.cpp:server-cuda` | NVIDIA A5000 (`device_ids: ["0"]`) | 30080 |
| `tei-embed` | `ghcr.io/huggingface/text-embeddings-inference:86-1.8` | CPU (no passthrough) | 8081 |
| `tei-rerank` | `ghcr.io/huggingface/text-embeddings-inference:86-1.8` | CPU (no passthrough) | 8082 |
| `searxng` | `searxng/searxng:latest` | CPU | 4000 |

All volumes (`llama-models`, `tei-embed-cache`, `tei-rerank-cache`, `searxng-data`) are owned by the infra stack.

### docker-compose.yml (researcher)

| Service | Notes |
|---------|-------|
| `researcher` | Rust binary; joins `ai-infra-net`; no `depends_on` (infra is external) |

## GPU Pinning Details

### NVIDIA A5000 — llama-cpp

```yaml
deploy:
  resources:
    reservations:
      devices:
        - driver: nvidia
          device_ids: ["0"]
          capabilities: [gpu]
```

### TEI services — CPU only

Neither TEI nor infinity (the main alternative) have reliable Docker images for consumer RDNA3 GPUs. TEI publishes no ROCm images at all; infinity's ROCm image targets MI200/MI300 data center cards. The models are small enough (bge-large ~300MB, cross-encoder ~90MB) that CPU latency is acceptable (20-50ms per batch) for the research pipeline. No device passthrough needed.

## Model Upgrade

With TEI off the A5000, usable VRAM is ~22GB. Target model: **Qwen3-30B Q4_K_M** (~20GB).

| Setting | Old | New |
|---------|-----|-----|
| Model | Qwen3.5-9B Q4_K_M (~5.5GB) | Qwen3-30B Q4_K_M (~20GB) |
| `LLAMA_CTX_SIZE` | 8192 | 16384 |
| `LLAMA_PARALLEL` | 4 | 2 (larger KV cache per slot) |

`STRIP_THINKING_TOKENS` still applies (same Qwen3 family).

## Operations

Root `Makefile` with targets: `infra-up`, `infra-down`, `infra-logs`, `infra-pull`, `up`, `down`, `logs`, `stop-all`.

**Boot order:** infra stack up first, then researcher. Enforced by workflow (or `@reboot` cron / systemd unit pointing at `infra/`). No Docker-level health-check coupling across stacks.

**`.env` split:**
- `infra/.env` — `COMPOSE_PROJECT_NAME=ai-infra`, model path, GPU layers, ctx size
- `.env` — `LLM_BASE_URL`, research params, API keys

## Constraints & Notes

- `depends_on` removed from researcher compose — infra is external, Docker cannot health-check across projects
- `COMPOSE_PROJECT_NAME=ai-infra` in `infra/.env` prevents name collisions with root compose project
- TEI runs on CPU only — no AMD GPU passthrough (no reliable consumer RDNA3 Docker support in TEI or infinity)
- AMD RX 7800 XT handles display only
