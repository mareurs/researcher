# researcher MCP — intermittent tool-registry drop in Claude Code

Status: active
Tags: bug, mcp, claude-code, investigation
Owner: marius
Created: 2026-05-15

## Symptom

Mid-session, the `researcher` stdio MCP server's tools disappear from Claude Code's tool list while the server-instructions block remains in the system prompt. Subagents spawned after the drop inherit a tool list without `mcp__researcher__*` entries — they see the instructions referencing the tools and report "no tools available."

Codescout (also stdio MCP) does NOT show the same behavior in the same sessions.

## Evidence

### Session-transcript evidence (2026-05-15)

Collected from CC session transcripts at `~/.claude/projects/-home-marius-work-*/`:

- **Frequency**: 15/77 (~19%) historical research-subagent invocations completed with zero `mcp__researcher__*` calls.
- **Direct subagent quote** (`agent-a4111f7a166abd629.jsonl`, project mirela-backend-kotlin):
  > "The researcher MCP tool was declared in the system reminder but isn't actually exposed in my function list. I cannot invoke `mcp__researcher__research` directly — it's not in my toolset."
- **Same-session asymmetry**: that failing subagent had `mcp__codescout__run_command` available and used it. Researcher tool absent, codescout present.
- **Parent session text** (same project, parent transcript `5359e112-...jsonl`):
  > "researcher keeps dropping. Likely a transport timeout on the researcher server process."
  > "researcher server appears in the MCP server manifest but no `mcp__researcher__*` tools are bound to my function table this session."
- **Process state on host**: `researcher-mcp` processes are alive (`ps aux`) when CC reports drop. Server binary did not crash.
- **`claude mcp list`**: server still shows `✓ Connected` from the CC CLI vantage point.

### tools/list capture (2026-05-15)

Captured raw `tools/list` JSON from both servers (`/tmp/r-resp.jsonl`, `/tmp/c-resp.jsonl`) by piping `initialize` + `notifications/initialized` + `tools/list` into the binary.

**Sizes (rules out hypothesis 2 — latency/size timeout):**
- Researcher `tools/list`: 2,264 bytes, 1 tool
- Codescout `tools/list`: 31,142 bytes, 25 tools
- Codescout is 14× larger and works fine — researcher is NOT being dropped for being slow or large.

**Schema differences (strong evidence for hypothesis 3 — schema-validation edge case):**

| Construct | Researcher | Codescout |
|---|---|---|
| `"type":["string","null"]` (nullable union) | 11 occurrences | 0 |
| `"type":["array","null"]` | 2 | 0 |
| `"type":["integer","null"]` | 2 | 0 |
| `"$schema"` declared inside `inputSchema` | yes | no |
| `"default": null` on optional fields | yes | no |
| `"format": "uint"` (non-standard JSON Schema format) | yes | no |
| `"format": "int64"` | no | yes (standard) |

Sample researcher field schema:
```json
"max_queries": {
  "default": null,
  "description": "max sub-questions for planner (topic/market only)",
  "format": "uint",
  "minimum": 0,
  "type": ["integer", "null"]
}
```

Sample codescout field schema (clean):
```json
"limit": { "default": 50, "maximum": 500, "type": "integer" }
```

**Why this is suspect:** Claude Code parses tool schemas with Zod. Zod's `z.union([...])` for type arrays exists but is brittle; Anthropic's prior SDKs have rejected `["T","null"]` form in favor of explicit `anyOf` or omitted-type-with-default. `"format": "uint"` is not in the JSON Schema spec and may trip strict parsers. Issue #57315 reports Zod-parse failures producing exactly the symptom we see.

**Origin of bad schema:** rmcp 1.1's `#[tool]` macro auto-derives `inputSchema` from the `ResearchInput` struct via `schemars`. `Option<T>` fields become `["T","null"]` unions; `usize` fields become `format: "uint"`. This is a `schemars` idiom that doesn't match Anthropic's preferred shape.
## Ruled out

- **Server crash**: process still running when drop occurs.
- **MCP-server side timeout**: `src/mcp_server.rs` uses bare `serve(stdio())` with no idle-timeout logic; rmcp 1.1's stdio transport doesn't impose one.
- **Subagent-config filter**: the same `general-purpose` subagent type successfully called `mcp__researcher__research` 62/77 historical times — propagation works when parent is bound. The drop is not a static config exclusion.
- **CC `notifications/tools/list_changed` re-fetch as mitigation**: confirmed NOT implemented in CC (anthropics/claude-code#13646, May 2026 — Zod schema present, handler stub). Server emitting `notify_tool_list_changed` would be silently discarded.

## Hypotheses (ranked)

1. **Schema-validation rejection (PROMOTED — top candidate).** CC's Zod-based tool-input parser rejects researcher's `inputSchema` on one of: `["T","null"]` type unions, `format: "uint"`, `default: null`, or `$schema` declared inside `inputSchema`. The server registration is accepted (server shows connected), the instructions block gets injected, but the tool's `inputSchema` fails Zod validation and the tool is silently dropped from the model's function list. Codescout's clean schemas pass. Matches symptom precisely. Related upstream: anthropics/claude-code#57315.

2. **CC client-side stale tool cache after transient transport blip.** CC fetches `tools/list` once at server init. If the stdio pipe stalls/recovers, CC doesn't re-fetch; tools stay missing even though `claude mcp list` reports connected. Still plausible as a secondary mechanism, but the schema explanation is simpler and matches the asymmetry better.

3. **Long stdio read latency on a single `tools/list` response trips an undocumented CC timeout.** RULED OUT — codescout is 14× larger and works fine.

4. **CC process restart / reconnect logic re-initializes some MCP servers but not researcher.** Researcher's stdio binary path or env vars might mismatch on reconnect.

5. **Race condition in CC plugin/instructions loading.** Server instructions land in system prompt before `tools/list` completes; if `tools/list` arrives "late" (>N ms), CC commits the system prompt without the tool entries.
## Diagnostic next steps

- [x] **Time `tools/list` response.** Done 2026-05-15. Researcher 2.2KB vs codescout 31KB. Size/latency ruled out.
- [x] **Compare schemas researcher vs codescout.** Done 2026-05-15. Researcher uses 15 nullable type unions, `format: "uint"`, `$schema` inside `inputSchema`. Top hypothesis.
- [x] **Test fix #1: clean schema generation.** Implemented 2026-05-15 via Path A (post-process). New free functions `sanitize_router_schemas()` + recursive `clean_schema_value()` in `src/mcp_server.rs`; `ResearcherServer::new` calls the sanitizer once after `Self::tool_router()` so the macro-generated `list_tools` ships the cleaned `inputSchema`. No `ResearchInput` struct refactor required (`Option<T>` left intact for serde-side optionality). Cleaner rules: drop top-level + nested `$schema`, drop `default: null`, `format: "uint"` → `minimum: 0` only, `type: ["X","null"]` → `type: "X"`. Verified by replaying `initialize` + `tools/list` against the rebuilt binary — all four reject markers (`["X","null"]`, `format:"uint"`, `$schema`, `default:null`) absent from output. **Drop-rate validation across sessions still pending**: needs `/mcp` reconnect + ≥20 `/research-subagent` invocations across multiple project CWDs to measure new failure rate vs 19% baseline.
- [ ] **If schema fix doesn't eliminate drops:** Capture raw MCP traffic via stdio-tee wrapper. Look for late `tools/list_changed` notifications, partial JSON, parse errors logged by CC.
- [ ] **Run CC with `--mcp-debug`** during repro session if the option exists in current build.
- [ ] **Reproduce idle-timeout angle.** Open a session, leave researcher idle 30/60/120 min, then check `mcp__researcher__research` visibility. Confirms or kills hypothesis 2/secondary mechanism.
- [ ] **Subscribe to upstream issues.** Watch anthropics/claude-code#13646 and #57315.
## Current mitigations in place

- `/home/marius/.claude/skills/research-subagent/SKILL.md` step 5: pre-spawn probe — parent verifies `mcp__researcher__research` is in its function list before spawning a subagent. If unbound, instructs user to `/mcp` reconnect rather than waste subagent spawn cost.
- Schema sanitizer (`mcp_server.rs:145-200`) strips Zod-hostile fields (`$schema`, `title`, `default:null`, `format:"uint"`→`minimum:0`, collapses `["T","null"]` unions) before router serves `tools/list`.
- rmcp 1.5 — protocol `2025-11-25` negotiated, `#[tool_handler(router = self.tool_router)]` so macro consumes the sanitized router stored on `ResearcherServer`.
- `serverInfo.name = "researcher"` (was rmcp default `"rmcp"`) — eliminates potential CC server-keying collision with codescout.
- Stdio tee (`TrafficLog` + `LoggedStdin/Out`) gated by `MCP_LOG_TRAFFIC=<path>` — captures full handshake for diagnosis.
## Workarounds (user-facing)

- `/mcp` and reconnect `researcher` server when drop is detected.
- Last resort: restart Claude Code session.

## Related artifacts

- Skill: `~/.claude/skills/research-subagent/SKILL.md` (pre-spawn probe)
- Server: `src/mcp_server.rs` (rmcp 1.1, bare stdio)
- Upstream issues: anthropics/claude-code#13646, #57315

## History

### 2026-05-15 — Tracker created

Investigation began from user report that `/research-subagent` "sometimes returns no findings." Systematic debugging of CC session transcripts surfaced the 19% failure rate and the parent/subagent asymmetry. Confirmed CC ignores `notifications/tools/list_changed` (#13646). Skill-side pre-spawn probe shipped; server-side mitigation deferred until raw MCP traffic capture confirms root cause.


### 2026-05-15 — tools/list capture done; new top hypothesis

Captured raw `tools/list` from both researcher and codescout via direct stdio piping. Researcher 2.2KB / 1 tool, codescout 31KB / 25 tools — size hypothesis dead. **Researcher's schema contains 15 nullable type unions, `format:"uint"`, and `$schema` declared inside `inputSchema`** — none present in codescout. Promoted "schema-validation rejection" to top hypothesis. Next action: ship a researcher build with clean schemas, measure drop rate change.

### 2026-05-15 — schema sanitizer shipped

Implemented Path A (post-process at startup, no struct refactor). Added `sanitize_router_schemas()` and recursive `clean_schema_value()` in `src/mcp_server.rs`; `ResearcherServer::new` drains the tool router, rewrites each `route.attr.input_schema`, and re-adds routes before storing. Verified rebuilt `target/release/researcher-mcp` emits clean shapes via direct stdio replay:

```
max_queries: {"description":"...","minimum":0,"type":"integer"}
domains:     {"description":"...","items":{"type":"string"},"type":"array"}
target:      {"description":"...","type":"string"}
```

All four CC-suspect markers (`["X","null"]`, `format:"uint"`, `$schema` inside inputSchema, `default:null`) now absent. Pending: `/mcp` reconnect + cross-session drop-rate measurement against the 15/77 baseline.

### 2026-05-15 — schema clean, tool still dropped → `title` field added to sanitizer

After fresh CC session restart with clean schema (no nullable unions, no `format:"uint"`, no `default:null`, no `$schema`), `mcp__researcher__*` tools were *still* absent from CC's function table. Subagent probe returned `none`.

Structural diff vs codescout's tools/list response surfaced one remaining asymmetry: **researcher's `inputSchema` top-level had a `title: "ResearchInput"` key** (schemars derive default). Codescout's `inputSchema` has only `{type, required, properties}` — no `title`. Added `title`-stripping to `clean_schema_value()` with a String-value guard so it never collides with a property literally named `"title"` (which has Object value).

Rebuilt. Verified `inputSchema` keys are now exactly `['properties', 'required', 'type']`. Re-test pending: `/mcp` reconnect + fresh subagent probe; if still absent, capture live CC↔server stdio via stdio-tee wrapper to inspect what CC actually sends as well as what it does after receiving the response.

### 2026-05-15 — live traffic capture; rmcp 1.5 upgrade; serverInfo.name

Wired stdio-tee in `src/mcp_server.rs` (`LoggedStdin`/`LoggedStdout` + `TrafficLog`, env-gated by `MCP_LOG_TRAFFIC=<path>`). Found project-level `.mcp.json` overrides global `~/.claude-kat/.claude.json` for same-named servers — env vars must be added in BOTH for hands-off testing.

Captured one full CC ↔ researcher handshake. Key observations:

```
→ initialize {protocolVersion: "2025-11-25", clientInfo.name: "claude-code", version: "2.1.142"}
← initialize {protocolVersion: "2025-06-18", serverInfo: {name: "rmcp", version: "1.1.0"}}
→ notifications/initialized
→ tools/list
← tools/list {tools: [{name: "research", inputSchema: {properties, required, type}}]}    # clean
(CC silent — no follow-up, no tools/call, no errors)
```

Two new diffs vs codescout-in-same-session (codescout's tools were visible):

1. **Protocol downgrade**: CC asked for 2025-11-25, rmcp 1.1 served 2025-06-18. rmcp 1.5 is first version supporting 2025-11-25.
2. **`serverInfo.name == "rmcp"` (rmcp default `from_build_env()`)**. Codescout ALSO reported `"rmcp"`, but: codescout's tools are sorted by tool-name length and exposed alphabetically — its tools appear in CC. The constant `"rmcp"` for serverInfo.name is unusual; CC may key tool-binding state by `serverInfo.name`, so two servers reporting the same name could conflict.

Both addressed in one pass:

- `Cargo.toml`: bumped `rmcp = "1.5"`, `cargo update`. Latest registry is rmcp-macros 1.5 + rmcp 1.5; protocol enum gains `V_2025_11_25` (`LATEST`).
- `Cargo.toml` release profile: `lto = "fat"`, `panic = "abort"`, `strip = "symbols"` — binaries shrank from 8.3M → 7.0M (researcher) / 7.3M (researcher-mcp).
- rmcp 1.5 `#[tool_handler]` calls `Self::tool_router()` (free fn) by default — bypassed our stored-and-sanitized field. Switched to `#[tool_handler(router = self.tool_router)]` so the macro consumes the cleaned router we stored in `ResearcherServer::new`.
- `get_info()` rewritten to use rmcp 1.5 builder: `ServerInfo::new(caps).with_server_info(Implementation::new("researcher", env!("CARGO_PKG_VERSION"))).with_instructions(...)`.

Wire verification of new binary:
```
protocolVersion: 2025-11-25
serverInfo: {name: "researcher", version: "0.1.0"}
inputSchema top keys: ['properties', 'required', 'type']    # still clean
```

Next: user must FULL CC restart (no `--resume` — that reuses launch-time MCP env cache) to force fresh tool binding. Subagent probe `mcp__researcher__*` will tell us whether protocol-version or serverInfo.name was the missing piece, or whether a deeper mechanism remains.


### 2026-05-15 — stale child after rebuild is a distinct failure mode

After full CC restart in a fresh terminal, traffic log captured a clean handshake (sanitized schema, protocolVersion 2025-11-25, serverInfo.name=`researcher`) — tool bound and call succeeded. But **other CC sessions still running from before the rebuild kept their old child PIDs alive** (verified via `readlink /proc/$PID/exe` → `target/release/researcher-mcp (deleted)`). Those children serve pre-sanitizer schemas → CC drops the tool. CC does **not** respawn the stdio child when the binary on disk changes.

New ops procedure: after `cargo build --release` of `researcher-mcp`, every active CC session that has it bound must either (a) `/mcp` reconnect (respawns child off new binary) or (b) be killed via `pkill -f target/release/researcher-mcp` followed by `/mcp` reconnect.

This finding turned the lights on for the original symptom: the schema-validation hypothesis was correct, but the visible failure mode in interactive use was "fix shipped, restart-flow incomplete" — not a residual schema bug. Confirmed end-to-end working: `pkill -f` of all `(deleted)`-exe children → `/mcp` reconnect → `mcp__researcher__research` available and live call returned 16 sources.
