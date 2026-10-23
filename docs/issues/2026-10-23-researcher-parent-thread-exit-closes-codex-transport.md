---
id: e5ba0f1e3d0a9755
kind: bug
status: fixed
title: researcher parent-death signal kills MCP server when spawning thread exits
owners:
- marius
tags:
- researcher
- codex
closed: 2026-10-23
opened: 2026-10-23
owner: marius
severity: high
---

# BUG: researcher dies when Codex's spawning thread exits

## Summary

External integration defect discovered while diagnosing researcher MCP in Codex. No configuration or implementation changed.

## Class fit

Unclassified external lifecycle interaction: Linux parent-death signals follow the spawning thread, while the intended lifetime is the whole Codex parent process. This is not a codescout transport defect.

## Symptom (Effect)

The registered researcher tool returns `Transport closed` after successful startup.

## Reproduction

Launch the configured researcher binary using Python subprocess.Popen inside a worker thread. Complete MCP initialize and notifications/initialized, keep stdin open, then let that thread finish while the main Python process remains alive. Compare with an otherwise identical child spawned from the main thread.

## Environment

Researcher source HEAD inspected: `f862f4f0ac9fbea357f035900b917748b6f20a73`. Probes used the existing release binary; it was not rebuilt, so source HEAD is not asserted to be the binary's build revision.

2026-10-23, Linux, active Codex configuration `/home/marius/.codex/config.toml`. Researcher binary `/home/marius/work/claude/researcher/target/release/researcher-mcp`.

## Ownership

Researcher MCP owns the defect. Its cleanup code intends to track the parent process, but PR_SET_PDEATHSIG tracks the creating thread. Codex launching a child from a blocking-pool thread is valid process-spawning behavior and exposes this assumption; no Codex protocol or lifecycle violation was established. Fix the researcher lifecycle guard rather than requiring clients to keep arbitrary spawning threads alive.

The Linux manual explicitly documents the thread-specific semantics: https://man7.org/linux/man-pages/man2/PR_SET_PDEATHSIG.2const.html (DESCRIPTION and CAVEATS).

## Root cause

Researcher `src/mcp_server.rs:902` sets `prctl(PR_SET_PDEATHSIG, SIGTERM)`. Codex `codex-rs/rmcp-client/src/stdio_server_launcher.rs:207` launches through `tokio::task::spawn_blocking`. A temporary launching thread can retire while its process remains alive, terminating researcher.

## Evidence

Measured 2026-10-23: Python subprocess.Popen from a worker thread, MCP initialization, then worker exit while parent stays alive:

```text
initialized True
initialized True
BEFORE spawning thread exit: control= None worker child= None
AFTER spawning thread exit: parent process alive= 1874013 control= None worker child= -15
```

Controlled live probe: both children completed initialization. Before worker exit, both poll() values were None. After worker exit, main-thread control remained alive and worker-spawned researcher returned -15 (SIGTERM), with Python parent alive. Codex SQLite logs show successful researcher initialization around 04:07:35 UTC and input-stream termination at 04:07:53–54, before the failed tool call at 04:08:54. The exact Codex worker-thread exit was not traced; its mechanism was independently reproduced.

## Hypotheses tried

Missing MCP registration disproved: tool is exposed and direct handshake succeeds. Idle shutdown is disabled (`idle_timeout_secs=0`).

## Fix

Replaced `PR_SET_PDEATHSIG` with a Linux parent-process watcher. It captures `getppid()`, exits immediately if already reparented to PID 1, and polls once per second for a changed parent process ID. A launching thread exiting inside a live parent leaves that ID unchanged. Reparenting to either init or a subreaper after watcher startup triggers shutdown, preserving orphan cleanup independently of stdin EOF.

## Verification status

Implemented and verified in the working tree on 2026-10-23; no fix commit has been created yet. Independent review applied four mutations across both fixes: all four were caught, zero survived. A no-op parent watcher failed the orphan-cleanup test; restoring the old parent-death signal failed the launching-thread test with SIGTERM. Archive after the fix is committed on master under the repository's archive rules.

## Tests added

`tests/mcp_lifecycle.rs::server_survives_spawning_thread_exit` launches the real MCP server from a Rust worker thread, completes initialization, exits the worker while preserving stdin, waits two seconds and verifies a successful MCP ping. Before the fix this test failed with SIGTERM (Unix wait status 15); after the fix it passes.

`server_exits_when_parent_process_dies` uses a subprocess fixture that leaves MCP stdin open in a pipe-holder process after the MCP parent exits. It verifies the server shuts down within five seconds while the holder remains alive, preventing stdin EOF from masking a missing parent watcher.

Verified 2026-10-23: full `cargo test`, `cargo check`, `cargo build --release`, and `git diff --check` passed. A fresh release server launched from a Python ThreadPoolExecutor worker survived executor shutdown, answered ping after two seconds, and completed live Vertex research. Clippy with `-D warnings` and the repository-wide formatting gate remain blocked by existing issues outside this fix.

## Workarounds

N/A — the rebuilt release binary contains the lifecycle fix. Existing MCP clients must reconnect/restart their researcher connection to load it.

## Resume

N/A — fix implemented and verified. Reconnect existing MCP clients to load the rebuilt release binary.

## References

Companion failure: [Vertex failure](2026-10-23-researcher-vertex-malformed-planner-response.md).
