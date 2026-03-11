# Dual-Model Routing Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Route pipeline LLM calls to a fast Qwen3-4B on AMD GPU (planner, summarizer) and keep the heavy Qwen3.5-27B on NVIDIA A5000 for final report generation only.

**Architecture:** Add a second llama.cpp container (`llama-cpp-fast`) using the ROCm image on the RX 7800 XT. The Rust code gets a second `LlmClient` instance (`new_fast`) that reads `LLM_FAST_*` env vars, with fallback to the heavy backend when unconfigured. Pipeline stages pass the appropriate client.

**Tech Stack:** Docker Compose (ROCm image), Rust (existing LlmClient, Config, pipeline), llama.cpp server-rocm.

**Spec:** `docs/superpowers/specs/2026-03-11-dual-model-routing-design.md`

---

## Chunk 1: Infrastructure — llama-cpp-fast container

### Task 1: Download Qwen3-4B model

**Files:**
- None (model download only)

- [ ] **Step 1: Download the model**

```bash
huggingface-cli download bartowski/Qwen3-4B-GGUF \
  --include "Qwen3-4B-Q4_K_M.gguf" \
  --local-dir /home/marius/.lmstudio/models/bartowski/Qwen3-4B-GGUF/
```

Expected: file at `/home/marius/.lmstudio/models/bartowski/Qwen3-4B-GGUF/Qwen3-4B-Q4_K_M.gguf` (~2.5GB).

- [ ] **Step 2: Verify file exists**

```bash
ls -lh /home/marius/.lmstudio/models/bartowski/Qwen3-4B-GGUF/Qwen3-4B-Q4_K_M.gguf
```

Expected: file present, ~2.5GB.

---

### Task 2: Add llama-cpp-fast to infra/docker-compose.yml

**Files:**
- Modify: `infra/docker-compose.yml`

- [ ] **Step 1: Add the llama-cpp-fast service**

Add this service block after the existing `llama-cpp` service (before `tei-embed`):

```yaml
  # ── llama.cpp fast: lightweight LLM on AMD RX 7800 XT via ROCm ────────────
  # Handles structured tasks (planner JSON, summarizer JSON) that don't need
  # a large model. Frees A5000 capacity for heavy report generation.
  llama-cpp-fast:
    image: ghcr.io/ggml-org/llama.cpp:server-rocm
    container_name: llama-cpp-fast
    networks:
      - ai-infra-net
    ports:
      - "30081:8080"
    volumes:
      - ${LLAMA_MODELS_PATH:-/home/marius/.lmstudio/models}:/models:ro
    environment:
      - LLAMA_ARG_MODEL=/models/${LLAMA_FAST_MODEL:-bartowski/Qwen3-4B-GGUF/Qwen3-4B-Q4_K_M.gguf}
      - LLAMA_ARG_HOST=0.0.0.0
      - LLAMA_ARG_PORT=8080
      - LLAMA_ARG_CTX_SIZE=${LLAMA_FAST_CTX_SIZE:-8192}
      - LLAMA_ARG_N_GPU_LAYERS=${LLAMA_FAST_GPU_LAYERS:-99}
      - LLAMA_ARG_PARALLEL=${LLAMA_FAST_PARALLEL:-4}
      - LLAMA_ARG_CONT_BATCHING=1
      - LLAMA_ARG_FLASH_ATTN=1
    devices:
      - /dev/kfd:/dev/kfd
      - /dev/dri:/dev/dri
    group_add:
      - video
      - render
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:8080/health"]
      interval: 15s
      timeout: 5s
      retries: 10
      start_period: 60s
    restart: unless-stopped
```

Also update the file header comment (lines 5-7) to reflect the new GPU assignment:

```
#   NVIDIA RTX A5000  → llama-cpp (heavy model, device_ids: ["0"])
#   AMD RX 7800 XT    → llama-cpp-fast (fast model, /dev/kfd + /dev/dri via ROCm)
#   CPU               → tei-embed, tei-rerank, searxng
```

Key differences from `llama-cpp` (heavy):
- Image: `server-rocm` instead of `server-cuda`
- GPU: `/dev/kfd` + `/dev/dri` (ROCm) instead of `device_ids: ["0"]` (NVIDIA)
- `group_add: [video, render]` required for ROCm device access
- Port: 30081 instead of 30080
- Smaller model, more parallel slots, smaller context

- [ ] **Step 2: Validate YAML syntax**

```bash
docker compose -f infra/docker-compose.yml config --quiet
```

Expected: no output, exit code 0.

- [ ] **Step 3: Commit**

```bash
git add infra/docker-compose.yml
git commit -m "feat: add llama-cpp-fast service (Qwen3-4B on AMD RX 7800 XT via ROCm)"
```

---

### Task 3: Update infra/.env.example with fast model vars

**Files:**
- Modify: `infra/.env.example`

- [ ] **Step 1: Add fast model section**

Append after the existing llama.cpp section:

```bash
# ── llama.cpp fast (AMD RX 7800 XT via ROCm) ─────────────────────────────────
# Lightweight model for structured pipeline tasks (planner, summarizer).
LLAMA_FAST_MODEL=bartowski/Qwen3-4B-GGUF/Qwen3-4B-Q4_K_M.gguf
LLAMA_FAST_CTX_SIZE=8192
LLAMA_FAST_GPU_LAYERS=99
LLAMA_FAST_PARALLEL=4
```

- [ ] **Step 2: Copy new vars to infra/.env**

```bash
cat >> infra/.env << 'EOF'

# ── llama.cpp fast (AMD RX 7800 XT via ROCm) ─────────────────────────────────
LLAMA_FAST_MODEL=bartowski/Qwen3-4B-GGUF/Qwen3-4B-Q4_K_M.gguf
LLAMA_FAST_CTX_SIZE=8192
LLAMA_FAST_GPU_LAYERS=99
LLAMA_FAST_PARALLEL=4
EOF
```

- [ ] **Step 3: Validate with env file**

```bash
docker compose -f infra/docker-compose.yml --env-file infra/.env config --quiet
```

Expected: exit code 0.

- [ ] **Step 4: Commit**

```bash
git add infra/.env.example
git commit -m "chore: add LLAMA_FAST_* vars to infra/.env.example"
```

---

### Task 4: Smoke test — llama-cpp-fast container

- [ ] **Step 1: Start the fast container**

```bash
docker compose -f infra/docker-compose.yml --env-file infra/.env up -d llama-cpp-fast
```

- [ ] **Step 2: Watch startup logs**

```bash
docker logs -f llama-cpp-fast
```

Wait for `server is listening on 0.0.0.0:8080`. If ROCm fails to detect the GPU, you'll see an error about `ggml_hip` or `no devices found` — see Notes for Implementer at the bottom.

- [ ] **Step 3: Verify health**

```bash
curl -s http://localhost:30081/health
```

Expected: `{"status":"ok"}` or similar HTTP 200 response.

- [ ] **Step 4: Test a completion**

```bash
curl -s http://localhost:30081/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{"model":"Qwen3-4B","messages":[{"role":"user","content":"Return JSON: {\"test\": true}"}],"max_tokens":50}' | python3 -m json.tool
```

Expected: JSON response with a `choices[0].message.content` containing `{"test": true}` or similar.

- [ ] **Step 5: Verify AMD GPU is being used**

```bash
sudo rocm-smi --showpidgpumem
```

Expected: a process using VRAM on GPU 0 (RX 7800 XT).

---

## Chunk 2: Rust code — Config + LlmClient + Pipeline routing

### Task 5: Add LLM_FAST_* fields to Config

**Files:**
- Modify: `src/config.rs` (Config struct, lines ~48-68)
- Modify: `src/mcp_server.rs` (config_from_env function, lines ~482-541)

- [ ] **Step 1: Add fields to Config struct**

Add these 4 fields after the existing `strip_thinking_tokens` field (line 68) and before the `searxng_url` field (line 73):

```rust
    /// Base URL for the fast/lightweight LLM backend (structured tasks).
    /// Empty string = fall back to LLM_BASE_URL.
    #[arg(long, env = "LLM_FAST_BASE_URL", default_value = "")]
    pub llm_fast_base_url: String,

    /// Model name for the fast LLM backend
    #[arg(long, env = "LLM_FAST_MODEL", default_value = "Qwen3-4B-Q4_K_M")]
    pub llm_fast_model: String,

    /// API key for the fast LLM backend. Empty = fall back to LLM_API_KEY.
    #[arg(long, env = "LLM_FAST_API_KEY", default_value = "")]
    pub llm_fast_api_key: String,

    /// Max tokens for fast LLM responses (structured JSON, typically small)
    #[arg(long, env = "LLM_FAST_MAX_TOKENS", default_value = "2048")]
    pub llm_fast_max_tokens: u32,
```

- [ ] **Step 2: Add fields to config_from_env()**

In `src/mcp_server.rs`, inside the `config_from_env()` function's `Config { ... }` block, add after `strip_thinking_tokens`:

```rust
        llm_fast_base_url: env("LLM_FAST_BASE_URL", ""),
        llm_fast_model: env("LLM_FAST_MODEL", "Qwen3-4B-Q4_K_M"),
        llm_fast_api_key: env("LLM_FAST_API_KEY", ""),
        llm_fast_max_tokens: env_usize("LLM_FAST_MAX_TOKENS", 2048) as u32,
```

- [ ] **Step 3: Verify it compiles**

```bash
cargo check 2>&1 | tail -5
```

Expected: `Finished` with no errors. There will be warnings about unused fields — that's fine, they'll be used in the next task.

- [ ] **Step 4: Commit**

```bash
git add src/config.rs src/mcp_server.rs
git commit -m "feat: add LLM_FAST_* config fields for dual-model routing"
```

---

### Task 6: Add LlmClient::new_fast constructor

**Files:**
- Modify: `src/llm/client.rs` (impl LlmClient block, lines ~60-74)

- [ ] **Step 1: Update tracing import**

In `src/llm/client.rs` line 4, change:
```rust
use tracing::debug;
```
to:
```rust
use tracing::{debug, info};
```

- [ ] **Step 2: Add the new_fast constructor**

Add this method inside `impl LlmClient`, after the existing `new()` method (after line 74):

```rust
    /// Build a client for the fast/lightweight LLM backend.
    /// Falls back to the heavy backend if `LLM_FAST_BASE_URL` is empty.
    pub fn new_fast(cfg: &Config) -> Self {
        let use_fast = !cfg.llm_fast_base_url.is_empty();

        let base_url = if use_fast {
            &cfg.llm_fast_base_url
        } else {
            &cfg.llm_base_url
        };

        let api_key = if use_fast && !cfg.llm_fast_api_key.is_empty() {
            &cfg.llm_fast_api_key
        } else {
            &cfg.llm_api_key
        };

        let (model, max_tokens) = if use_fast {
            (&cfg.llm_fast_model, cfg.llm_fast_max_tokens)
        } else {
            (&cfg.llm_model, cfg.llm_max_tokens)
        };

        info!(
            backend = if use_fast { "fast" } else { "heavy (fallback)" },
            url = %base_url,
            model = %model,
            "LlmClient::new_fast"
        );

        Self {
            http: Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("HTTP client"),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: api_key.clone(),
            model: model.clone(),
            max_tokens,
            temperature: cfg.llm_temperature,
            strip_thinking: cfg.strip_thinking_tokens,
        }
    }
```

- [ ] **Step 3: Add info log to existing new() for symmetry**

Add a log line at the end of the existing `new()` constructor, just before closing `}` of the `Self { ... }` block. Actually, since `Self` is returned directly, add the log before `Self { ... }`:

```rust
    pub fn new(cfg: &Config) -> Self {
        info!(
            backend = "heavy",
            url = %cfg.llm_base_url,
            model = %cfg.llm_model,
            "LlmClient::new"
        );
        Self {
            // ... existing fields unchanged ...
        }
    }
```

- [ ] **Step 4: Verify it compiles**

```bash
cargo check 2>&1 | tail -5
```

Expected: `Finished` with no errors.

- [ ] **Step 5: Commit**

```bash
git add src/llm/client.rs
git commit -m "feat: add LlmClient::new_fast() with fallback to heavy backend"
```

---

### Task 7: Wire dual clients into pipeline

**Files:**
- Modify: `src/researcher/pipeline.rs` (run function, lines ~170-328)

- [ ] **Step 1: Create both clients in run()**

In `run()`, replace line 202:
```rust
    let llm = LlmClient::new(cfg);
```

with:
```rust
    let llm = LlmClient::new(cfg);
    let llm_fast = LlmClient::new_fast(cfg);
```

- [ ] **Step 2: Route planner to fast client**

Replace line 210:
```rust
    let queries = generate_queries(&llm, topic, max_queries, &domains, &request.target).await?;
```

with:
```rust
    let queries = generate_queries(&llm_fast, topic, max_queries, &domains, &request.target).await?;
```

- [ ] **Step 3: Route summarizer to fast client**

Replace line 309:
```rust
    let summaries = summarize_all(&llm, &sources, topic).await;
```

with:
```rust
    let summaries = summarize_all(&llm_fast, &sources, topic).await;
```

- [ ] **Step 4: Verify publisher still uses heavy client**

Line 319 should already read:
```rust
    let raw_report = write_report(&llm, topic, &summaries, &request.mode, &request.target, token_tx).await?;
```

This stays on `&llm` (heavy) — no change needed. Confirm it's unchanged.

- [ ] **Step 5: Verify it compiles**

```bash
cargo check 2>&1 | tail -5
```

Expected: `Finished` with no errors.

- [ ] **Step 6: Commit**

```bash
git add src/researcher/pipeline.rs
git commit -m "feat: route planner and summarizer to fast LLM, publisher to heavy"
```

---

## Chunk 3: App config + smoke test

### Task 8: Update docker-compose.yml and .env.example

**Files:**
- Modify: `docker-compose.yml`
- Modify: `.env.example`

- [ ] **Step 1: Add LLM_FAST_* env vars to docker-compose.yml**

In the `environment:` section of the `researcher` service, add after `STRIP_THINKING_TOKENS`:

```yaml
      # Fast LLM backend — llama-cpp-fast in infra stack (structured tasks)
      - LLM_FAST_BASE_URL=${LLM_FAST_BASE_URL:-http://llama-cpp-fast:8080/v1}
      - LLM_FAST_MODEL=${LLM_FAST_MODEL:-Qwen3-4B-Q4_K_M}
      - LLM_FAST_API_KEY=${LLM_FAST_API_KEY:-no-key-needed}
      - LLM_FAST_MAX_TOKENS=${LLM_FAST_MAX_TOKENS:-2048}
```

- [ ] **Step 2: Add LLM_FAST_* section to .env.example**

Add after `STRIP_THINKING_TOKENS=true`:

```bash
# ── Fast LLM Backend (structured tasks: planner, summarizer) ─────────────────
# Points to llama-cpp-fast in infra stack. Empty = fall back to heavy LLM.
LLM_FAST_BASE_URL=http://llama-cpp-fast:8080/v1
LLM_FAST_MODEL=Qwen3-4B-Q4_K_M
LLM_FAST_API_KEY=no-key-needed
LLM_FAST_MAX_TOKENS=2048
```

- [ ] **Step 3: Validate compose**

```bash
docker compose config --quiet
```

Expected: exit code 0.

- [ ] **Step 4: Commit**

```bash
git add docker-compose.yml .env.example
git commit -m "chore: add LLM_FAST_* env vars to researcher compose and .env.example"
```

---

### Task 9: Update CLAUDE.md env vars table

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Add LLM_FAST_* entries to the Env Vars Reference table**

Add these rows after the `STRIP_THINKING_TOKENS` row:

```markdown
| `LLM_FAST_BASE_URL` | `` (disabled) | Fast LLM endpoint; empty = use heavy backend |
| `LLM_FAST_MODEL` | `Qwen3-4B-Q4_K_M` | Model name for fast LLM |
| `LLM_FAST_API_KEY` | `` | Fast LLM API key; empty = use `LLM_API_KEY` |
| `LLM_FAST_MAX_TOKENS` | `2048` | Max tokens for fast LLM responses |
```

- [ ] **Step 2: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: add LLM_FAST_* env vars to CLAUDE.md reference table"
```

---

### Task 10: Build and end-to-end smoke test

- [ ] **Step 1: Build release binary**

```bash
cargo build --release 2>&1 | tail -3
```

Expected: `Finished release` with no errors.

- [ ] **Step 2: Verify both llama containers are healthy**

```bash
curl -s http://localhost:30080/health && echo " (heavy)"
curl -s http://localhost:30081/health && echo " (fast)"
```

Expected: both return HTTP 200.

- [ ] **Step 3: Run pipeline with dual routing**

```bash
RUST_LOG=info \
LLM_BASE_URL=http://localhost:30080/v1 \
LLM_MODEL=Qwen_Qwen3.5-27B-Q4_K_M \
LLM_MAX_TOKENS=16384 \
LLM_FAST_BASE_URL=http://localhost:30081/v1 \
LLM_FAST_MODEL=Qwen3-4B-Q4_K_M \
LLM_FAST_MAX_TOKENS=2048 \
SEARXNG_URL=http://localhost:4000 \
STRIP_THINKING_TOKENS=true \
EMBED_BASE_URL=http://localhost:8081 \
RERANK_BASE_URL=http://localhost:8082 \
./target/release/researcher --query "benefits of async Rust" --output /tmp/dual-model-test.md
```

Watch for log lines:
- `LlmClient::new backend="heavy" url=http://localhost:30080/v1`
- `LlmClient::new_fast backend="fast" url=http://localhost:30081/v1`
- Planner and summarizer should complete faster (small model, no queue contention)
- Publisher should still use heavy model

- [ ] **Step 4: Check output**

```bash
head -30 /tmp/dual-model-test.md
```

Expected: a markdown report with coherent content and source citations.

- [ ] **Step 5: Verify GPU usage during pipeline run**

Run in a separate terminal while the pipeline is active:

```bash
nvidia-smi --query-gpu=utilization.gpu --format=csv,noheader  # A5000 — spikes during report
sudo rocm-smi --showuse                                        # RX 7800 XT — active during planner/summarizer
```

---

### Task 11: Update MCP configs

**Files:**
- Modify: `~/.claude/.claude.json` (researcher MCP env)
- Modify: `~/.claude-sdd/.claude.json` (researcher MCP env)

- [ ] **Step 1: Add LLM_FAST_* to both MCP config files**

In both files, find the `researcher` MCP server's `env` block and add:

```json
"LLM_FAST_BASE_URL": "http://localhost:30081/v1",
"LLM_FAST_MODEL": "Qwen3-4B-Q4_K_M",
"LLM_FAST_MAX_TOKENS": "2048"
```

- [ ] **Step 2: Restart MCP and test**

Restart MCP (`/mcp` → restart researcher), then test:

```
mcp__researcher__research(query="what is WebAssembly", mode="summary")
```

Expected: a summary with source citations, completed faster than before.

---

## Notes for Implementer

**ROCm image fails to detect GPU?** The `server-rocm` image needs:
1. `/dev/kfd` and `/dev/dri` device passthrough
2. `group_add: [video, render]` for device permissions
3. The host user must be in the `render` group (already confirmed)

If the Docker image itself doesn't support gfx1101 (RDNA3), the fallback is to compile llama.cpp natively with ROCm:
```bash
git clone https://github.com/ggerganov/llama.cpp
cd llama.cpp
cmake -B build -DGGML_HIP=ON -DAMDGPU_TARGETS=gfx1101
cmake --build build --config Release -j$(nproc)
# Run directly:
./build/bin/llama-server -m /path/to/model.gguf --host 0.0.0.0 --port 8080
```

**Model not downloaded?** Task 1 covers the download. If HuggingFace CLI isn't installed: `pip install huggingface-hub`.

**Fallback if AMD is too slow or unstable:** Set `LLM_FAST_BASE_URL=` (empty) and both clients will use the A5000. Zero code changes needed.
