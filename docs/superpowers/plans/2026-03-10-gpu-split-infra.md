# GPU Split Infrastructure Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the monolithic `docker-compose.yml` into a shared `infra/` stack (llama-cpp on A5000, TEI on AMD ROCm, SearXNG on CPU) and a lean researcher app stack that joins infra via external Docker network.

**Architecture:** Two independent Docker Compose projects sharing an external bridge network (`ai-infra-net`). The infra stack owns all GPU services and volumes; the researcher stack is a single container that joins the network. GPU work is split: NVIDIA A5000 runs llama-cpp only, AMD RX 7800 XT (Navi 32) runs TEI embed + rerank via ROCm.

**Tech Stack:** Docker Compose v2, NVIDIA Container Toolkit (already installed), ROCm via `/dev/kfd` + `/dev/dri` device passthrough, TEI ROCm image, llama-cpp CUDA image.

**Spec:** `docs/superpowers/specs/2026-03-10-gpu-split-infra-design.md`

---

## Chunk 1: infra/ stack

### Task 1: Verify ROCm TEI image tag

**Files:**
- No file changes — discovery only

- [ ] **Step 1: Check available ROCm TEI tags**

```bash
curl -s "https://ghcr.io/v2/huggingface/text-embeddings-inference/tags/list" \
  2>/dev/null | python3 -m json.tool 2>/dev/null | grep rocm | head -20
```

If that returns nothing (auth required), check manually:
https://github.com/huggingface/text-embeddings-inference/releases

Look for the latest `rocm-*` tag. As of early 2026 it should be `rocm-1.6` or later.

- [ ] **Step 2: Note the tag**

Record the exact tag — you'll use it in Task 2. If `rocm-1.6` exists, use it. If not, use the latest `rocm-*` tag available.

---

### Task 2: Create infra/docker-compose.yml

**Files:**
- Create: `infra/docker-compose.yml`

- [ ] **Step 1: Create the file**

```yaml
# infra/docker-compose.yml
#
# Shared AI infrastructure stack — always-on, reusable across projects.
#
# GPU split:
#   NVIDIA RTX A5000  → llama-cpp (heavy LLM inference)
#   AMD RX 7800 XT    → tei-embed + tei-rerank (ROCm; small models, frees A5000 VRAM)
#   CPU               → searxng
#
# Bring up:   docker compose up -d
# Tear down:  docker compose down
# (Use root Makefile: make infra-up / make infra-down)

networks:
  ai-infra-net:
    driver: bridge

volumes:
  llama-models:
  searxng-data:
  tei-embed-cache:
  tei-rerank-cache:

services:
  # ── SearXNG: private metasearch ────────────────────────────────────────────
  searxng:
    image: searxng/searxng:latest
    container_name: searxng
    networks:
      - ai-infra-net
    ports:
      - "4000:8080"
    volumes:
      - searxng-data:/etc/searxng
      - ../config/searxng/settings.yml:/etc/searxng/settings.yml:ro
    environment:
      - SEARXNG_BASE_URL=http://localhost:4000/
    restart: unless-stopped

  # ── llama.cpp: OpenAI-compatible LLM, NVIDIA A5000 only ───────────────────
  # device_ids: ["0"] pins to the A5000 (GPU index 0).
  # Reusable: any project on ai-infra-net can use http://llama-cpp:8080/v1
  llama-cpp:
    image: ghcr.io/ggml-org/llama.cpp:server-cuda
    container_name: llama-cpp
    networks:
      - ai-infra-net
    ports:
      - "30080:8080"
    volumes:
      - /home/marius/.lmstudio/models:/models:ro
    environment:
      - LLAMA_ARG_MODEL=/models/${LLAMA_MODEL:-lmstudio-community/Qwen3-30B-GGUF/Qwen3-30B-Q4_K_M.gguf}
      - LLAMA_ARG_HOST=0.0.0.0
      - LLAMA_ARG_PORT=8080
      - LLAMA_ARG_CTX_SIZE=${LLAMA_CTX_SIZE:-16384}
      - LLAMA_ARG_N_GPU_LAYERS=${LLAMA_GPU_LAYERS:-99}
      - LLAMA_ARG_PARALLEL=${LLAMA_PARALLEL:-2}
      - LLAMA_ARG_CONT_BATCHING=1
      - LLAMA_ARG_FLASH_ATTN=1
    deploy:
      resources:
        reservations:
          devices:
            - driver: nvidia
              device_ids: ["0"]       # A5000 is GPU index 0
              capabilities: [gpu]
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 15s
      timeout: 5s
      retries: 10
      start_period: 60s
    restart: unless-stopped

  # ── TEI embed: AMD RX 7800 XT via ROCm ────────────────────────────────────
  # HSA_OVERRIDE_GFX_VERSION=11.0.0 is required for Navi 32 (gfx1101).
  # Without it ROCm 6.x may misidentify the GPU and refuse to run.
  tei-embed:
    image: ghcr.io/huggingface/text-embeddings-inference:rocm-1.6
    container_name: tei-embed
    networks:
      - ai-infra-net
    ports:
      - "8081:80"
    command: --model-id BAAI/bge-large-en-v1.5 --port 80 --auto-truncate --max-concurrent-requests 32
    volumes:
      - tei-embed-cache:/data
    devices:
      - /dev/kfd:/dev/kfd
      - /dev/dri/renderD128:/dev/dri/renderD128
    group_add:
      - video
      - render
    environment:
      - HSA_OVERRIDE_GFX_VERSION=11.0.0
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:80/health"]
      interval: 10s
      timeout: 5s
      retries: 10
      start_period: 90s
    restart: unless-stopped

  # ── TEI rerank: AMD RX 7800 XT via ROCm ───────────────────────────────────
  # Same ROCm setup as tei-embed. Cross-encoder is ~90MB — trivial VRAM cost.
  tei-rerank:
    image: ghcr.io/huggingface/text-embeddings-inference:rocm-1.6
    container_name: tei-rerank
    networks:
      - ai-infra-net
    ports:
      - "8082:80"
    command: --model-id cross-encoder/ms-marco-MiniLM-L-6-v2 --port 80 --auto-truncate --max-concurrent-requests 32
    volumes:
      - tei-rerank-cache:/data
    devices:
      - /dev/kfd:/dev/kfd
      - /dev/dri/renderD128:/dev/dri/renderD128
    group_add:
      - video
      - render
    environment:
      - HSA_OVERRIDE_GFX_VERSION=11.0.0
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:80/health"]
      interval: 10s
      timeout: 5s
      retries: 10
      start_period: 90s
    restart: unless-stopped
```

Replace `rocm-1.6` with the tag you verified in Task 1 if it differs.

- [ ] **Step 2: Validate YAML syntax**

```bash
docker compose -f infra/docker-compose.yml config --quiet
```

Expected: no output, exit code 0. Any error means a YAML syntax problem — fix before continuing.

- [ ] **Step 3: Commit**

```bash
git add infra/docker-compose.yml
git commit -m "feat: add infra/docker-compose.yml with GPU-split AI stack"
```

---

### Task 3: Create infra/.env.example

**Files:**
- Create: `infra/.env.example`

- [ ] **Step 1: Create the file**

```bash
# infra/.env.example — copy to infra/.env and edit
#
# COMPOSE_PROJECT_NAME isolates this stack from the root compose project.
# Without it, `docker compose` in the repo root might conflict with infra names.
COMPOSE_PROJECT_NAME=ai-infra

# ── llama.cpp ─────────────────────────────────────────────────────────────────
# Path relative to the /models volume mount (/home/marius/.lmstudio/models)
LLAMA_MODEL=lmstudio-community/Qwen3-30B-GGUF/Qwen3-30B-Q4_K_M.gguf
LLAMA_CTX_SIZE=16384
LLAMA_GPU_LAYERS=99     # 99 = all layers to GPU
LLAMA_PARALLEL=2        # 2 slots; 30B model needs ~10GB KV cache per slot at ctx=16384
```

- [ ] **Step 2: Copy to infra/.env and set actual model path**

```bash
cp infra/.env.example infra/.env
```

Edit `infra/.env`: verify `LLAMA_MODEL` matches the actual path under `~/.lmstudio/models/`.

```bash
ls ~/.lmstudio/models/ | grep -i qwen3
```

If Qwen3-30B isn't downloaded yet, update `LLAMA_MODEL` to your current best model and proceed — the model path can be changed later without touching any compose file.

- [ ] **Step 3: Validate infra compose with env file**

```bash
docker compose -f infra/docker-compose.yml --env-file infra/.env config --quiet
```

Expected: exit code 0.

- [ ] **Step 4: Commit**

```bash
git add infra/.env.example
git commit -m "feat: add infra/.env.example with llama-cpp tuning vars"
```

---

## Chunk 2: Root compose cleanup

### Task 4: Update docker-compose.yml (researcher only)

**Files:**
- Modify: `docker-compose.yml`

The root compose becomes minimal: one service, one external network, no volumes.

- [ ] **Step 1: Replace docker-compose.yml**

```yaml
# docker-compose.yml
#
# Researcher app stack — the Rust binary only.
# All infrastructure (LLM, TEI, SearXNG) lives in infra/docker-compose.yml.
#
# Prerequisites: infra stack must be running first.
#   cd infra && docker compose up -d
#   (or: make infra-up from repo root)
#
# Quick start:
#   docker compose up -d

networks:
  ai-infra-net:
    external: true    # owned by infra/docker-compose.yml

services:
  researcher:
    build:
      context: .
      dockerfile: Dockerfile
    container_name: researcher
    networks:
      - ai-infra-net
    ports:
      - "33100:3000"
    environment:
      # LLM backend — llama-cpp in infra stack, or swap for OpenAI
      - LLM_BASE_URL=${LLM_BASE_URL:-http://llama-cpp:8080/v1}
      - LLM_MODEL=${LLM_MODEL:-Qwen3-30B-Q4_K_M}
      - LLM_API_KEY=${OPENAI_API_KEY:-no-key-needed}
      - STRIP_THINKING_TOKENS=${STRIP_THINKING_TOKENS:-true}
      # Search
      - SEARXNG_URL=http://searxng:8080
      - SEARCH_RESULTS_PER_QUERY=${SEARCH_RESULTS_PER_QUERY:-8}
      # Embeddings (TEI embed in infra stack)
      - EMBED_BASE_URL=http://tei-embed:80
      - EMBED_MODEL=${EMBED_MODEL:-BAAI/bge-large-en-v1.5}
      - DEDUP_THRESHOLD=${DEDUP_THRESHOLD:-0.92}
      # Reranking (TEI rerank in infra stack)
      - RERANK_BASE_URL=http://tei-rerank:80
      # Research config
      - MAX_SEARCH_QUERIES=${MAX_SEARCH_QUERIES:-4}
      - MAX_SOURCES_PER_QUERY=${MAX_SOURCES_PER_QUERY:-4}
      - LLM_MAX_TOKENS=${LLM_MAX_TOKENS:-2048}
      - LLM_TEMPERATURE=${LLM_TEMPERATURE:-0.3}
      - RUST_LOG=${RUST_LOG:-info}
    restart: unless-stopped
```

Note: `depends_on` is intentionally absent — Docker cannot health-check across compose projects. Infra must be up before running `docker compose up`.

- [ ] **Step 2: Validate**

```bash
docker compose config --quiet
```

Expected: exit code 0.

- [ ] **Step 3: Commit**

```bash
git add docker-compose.yml
git commit -m "refactor: slim docker-compose.yml to researcher-only; infra moved to infra/"
```

---

### Task 5: Update root .env.example

**Files:**
- Modify: `.env.example`

Remove infra vars (model path, GPU layers, ctx size) — those now live in `infra/.env.example`.

- [ ] **Step 1: Replace .env.example**

```bash
# .env.example — researcher app tunables only.
# Infra tunables (LLAMA_MODEL, LLAMA_CTX_SIZE, etc.) are in infra/.env.example

# ── LLM Backend ───────────────────────────────────────────────────────────────
# Option A: Local llama-cpp (default — infra stack must be running)
LLM_BASE_URL=http://llama-cpp:8080/v1
LLM_MODEL=Qwen3-30B-Q4_K_M
LLM_API_KEY=no-key-needed
STRIP_THINKING_TOKENS=true

# Option B: OpenAI
# LLM_BASE_URL=https://api.openai.com/v1
# LLM_MODEL=gpt-4o-mini
# LLM_API_KEY=sk-...

# Option C: Ollama (already running locally)
# LLM_BASE_URL=http://host.docker.internal:11434/v1
# LLM_MODEL=qwen3:30b
# LLM_API_KEY=ollama

# ── Embeddings / deduplication ────────────────────────────────────────────────
EMBED_BASE_URL=http://tei-embed:80
EMBED_MODEL=BAAI/bge-large-en-v1.5
DEDUP_THRESHOLD=0.92

# ── Research tuning ───────────────────────────────────────────────────────────
MAX_SEARCH_QUERIES=4
MAX_SOURCES_PER_QUERY=4
SEARCH_RESULTS_PER_QUERY=8

# ── LLM generation ────────────────────────────────────────────────────────────
LLM_MAX_TOKENS=2048
LLM_TEMPERATURE=0.3

# ── Logging ───────────────────────────────────────────────────────────────────
RUST_LOG=info
```

- [ ] **Step 2: Commit**

```bash
git add .env.example
git commit -m "chore: remove infra vars from root .env.example (moved to infra/.env.example)"
```

---

## Chunk 3: Makefile + smoke test

### Task 6: Add root Makefile

**Files:**
- Create: `Makefile`

- [ ] **Step 1: Create Makefile**

```makefile
# Makefile — convenience targets for the two-stack setup.
# Prerequisites: infra stack must be up before running `make up`.

.PHONY: infra-up infra-down infra-logs infra-pull up down logs stop-all

## Infra stack (llama-cpp, tei-embed, tei-rerank, searxng)
infra-up:
	docker compose -f infra/docker-compose.yml --env-file infra/.env up -d

infra-down:
	docker compose -f infra/docker-compose.yml --env-file infra/.env down

infra-logs:
	docker compose -f infra/docker-compose.yml --env-file infra/.env logs -f

infra-pull:
	docker compose -f infra/docker-compose.yml --env-file infra/.env pull

## Researcher app
up:
	docker compose up -d

down:
	docker compose down

logs:
	docker compose logs -f

## Bring everything down (keeps volumes)
stop-all: down infra-down
```

- [ ] **Step 2: Commit**

```bash
git add Makefile
git commit -m "chore: add Makefile with infra-up/down/logs and up/down/logs targets"
```

---

### Task 7: Smoke test — infra stack

- [ ] **Step 1: Pull infra images**

```bash
make infra-pull
```

This will error if the TEI ROCm tag doesn't exist — fix the tag in `infra/docker-compose.yml` if so.

- [ ] **Step 2: Start infra stack**

```bash
make infra-up
```

- [ ] **Step 3: Watch startup logs**

```bash
make infra-logs
```

Wait for all four services to become healthy. TEI models download on first start (~1-2GB each) — expect 2-5 minutes. llama-cpp start_period is 60s.

- [ ] **Step 4: Verify network exists**

```bash
docker network ls | grep ai-infra-net
```

Expected: one line showing `ai-infra-net` with driver `bridge`.

- [ ] **Step 5: Verify service health**

```bash
curl -s http://localhost:4000/     | grep -i searx     # SearXNG UI
curl -s http://localhost:8081/health                   # TEI embed
curl -s http://localhost:8082/health                   # TEI rerank
curl -s http://localhost:30080/health                  # llama-cpp
```

All should return HTTP 200.

- [ ] **Step 6: Verify GPU assignment**

```bash
# A5000 should show llama-cpp process
nvidia-smi

# AMD card should show TEI processes
cat /sys/kernel/debug/dri/1/clients 2>/dev/null || \
  docker exec tei-embed rocm-smi 2>/dev/null || \
  echo "Check AMD GPU load via: watch -n1 radeontop"
```

- [ ] **Step 7: Commit smoke test outcome (if any config fixes were needed)**

If you had to fix the TEI image tag or any other config:

```bash
git add infra/docker-compose.yml
git commit -m "fix: correct TEI ROCm image tag to <actual-tag>"
```

---

### Task 8: Smoke test — researcher stack

- [ ] **Step 1: Start researcher**

```bash
make up
```

- [ ] **Step 2: Verify researcher joined the network**

```bash
docker network inspect ai-infra-net | grep researcher
```

Expected: `researcher` container listed under `Containers`.

- [ ] **Step 3: Quick research request**

```bash
curl -s -X POST http://localhost:33100/research \
  -H "Content-Type: application/json" \
  -d '{"query": "what is Rust", "mode": "quick"}' | head -c 500
```

Expected: JSON with a short research result. If TEI or llama-cpp aren't reachable, researcher will log errors — check with `make logs`.

- [ ] **Step 4: Final commit**

```bash
git add .
git commit -m "chore: final infra split smoke test complete"
```

---

## Notes for Implementer

**ROCm `/dev/kfd` permissions:** If TEI fails to open `/dev/kfd`, the Docker daemon user needs access. Check:
```bash
ls -la /dev/kfd
# If group is 'render': add the docker user to the render group
sudo usermod -aG render $USER  # then re-login
```

**Model not downloaded yet?** Update `LLAMA_MODEL` in `infra/.env` to an existing model path, start infra, verify everything works, then swap to Qwen3-30B once downloaded. No compose file changes needed — only the env file.

**HSA_OVERRIDE_GFX_VERSION:** If TEI still fails on AMD with `No GPU found`, try `HSA_OVERRIDE_GFX_VERSION=11.0.1` (exact Navi 32 sub-revision). Check with:
```bash
docker exec tei-embed rocminfo 2>/dev/null | grep gfx
```
