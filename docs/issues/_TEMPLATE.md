---
status: template
opened: YYYY-MM-DD
closed:
severity: medium
owner: marius
related: []
tags: []
kind: bug
---

<!--
BUG TRACKER TEMPLATE — do not edit content; copy this file.

To open a bug:
  1. Copy this file to docs/issues/$(date -I)-<slug>.md
  2. Replace this comment block with the bug content.
  3. Done — the librarian discovers the file on the next reindex via its
     `kind: bug` frontmatter. There is no rule needed in the registry and no
     manual index file. List active bugs with:
       artifact(action="find", kind="bug",
                filter={"status": {"in": ["open", "investigating"]}})
     Both non-terminal states — status="open" alone hides any bug marked
     `investigating`, which is what you set while actively working one.

Where this surface came from, and the one thing to keep true:
  researcher is a member of the `codescout-ecosystem` librarian umbrella, so
  a session in codescout finds these files with scope="umbrella" and vice
  versa. That is the whole reason researcher bugs no longer need to be filed
  in codescout's tracker. Umbrella membership lives in
  ~/.config/librarian/workspace.toml — per-machine, outside every repo, and
  deliberately NOT recorded in any committed file, because a host that does
  not have it would read this note as false. To check what actually resolves,
  run a scope="umbrella" query and read the returned `scope` block.

Cross-repo citations:
  Prefix them with the repo — `codescout:docs/trackers/foo.md`. An unprefixed
  path is researcher-relative. A bare cross-repo path silently reads as local
  and rots without any lint noticing.

Trigger rules — open a file for ANY bug noticed during work:
  ✓ User explicitly asks ("log this", "open a tracker")
  ✓ Bug blocking the current task (fix-now or parking-lot)
  ✓ Incidental bug we won't fix in the current session
  ✓ Just-fixed bug whose investigation is worth preserving
  ✓ Tool quirks / misbehaviors
  ✗ Pure typos / one-token corrections — commit message is enough
  ✗ Feature ideas / refactors — those go in docs/plans/ or docs/trackers/
  ✗ Subjective dislikes that aren't bugs

Status field semantics:
  open          — Logged, investigation not started or paused.
  investigating — Actively being worked on this session.
  fixed         — Root cause addressed, regression test added, verified.
  mitigated     — Workaround in place; root cause not addressed.
  wontfix       — Intentionally not fixing; justification in the file.
  zombie        — No longer observed but root cause not confirmed; kept open
                  in case it recurs. Pair with `last_observed:` in frontmatter
                  and a "Status: zombie" section documenting the re-open
                  trigger.
  `closed:` stays empty at creation — fill in YYYY-MM-DD only when status
  flips to fixed/mitigated/wontfix.

Archive trigger: move the file into docs/issues/archive/ once the fix is on
`master` with a green gate and a regression test, and record the fix SHA in
§ Fix. researcher is a single-branch repo — there is one `master`, no
promotion branch, so there is exactly one SHA per fix and nothing to re-cite
later. (If researcher ever grows a long-lived working branch, this paragraph
is the one that goes stale first.)
Archive via artifact(action="move", id=..., new_rel_path="docs/issues/archive/...")
— never a bare git mv, which orphans the catalog row: id = sha256(abs_path),
so a hand-move leaves the row pointing at a path that no longer exists.

Use `N/A` or `Unknown — under investigation` for sections that don't yet
apply. `N/A` in `Tests added` requires justification — an empty Tests added
without justification means the bug isn't really closed.
-->

# BUG: <one-line summary>

## Summary
*1–3 sentences. What's broken, who's affected, the elevator pitch.*

## Symptom (Effect)
*Capture the EXACT observable behavior. Verbatim error string in a code fence
(no paraphrasing). Exit code if any. Timing if relevant. What was observed,
not what it means.*

## Reproduction
*Minimal copy-pasteable steps. Include git commit (`git rev-parse HEAD`) and
how to invoke (`cargo run --release` / `/mcp` / the research() call and its
params). If not yet reproducible, write `Not yet reproducible — best lead: …`
and stop.*

## Environment
*OS, Rust version, MCP transport, which profile's config, which backend
(SearxNG / Vertex / …), which model and endpoint. For anything touching the
reranker or embedder, name the served model and the protocol — a URL alone
does not identify the response shape.*

## Root cause
*Mechanism, in mechanism-language ("X holds a lock while Y waits on it"), not
symptom-language. Cite `path:line` for every claim. If unknown, write
`Unknown — see Hypotheses tried` and link.*

*Also cite what **measured** it — the command and the date, on one line:
`measured 2026-08-07: curl -s localhost:30083/rerank … → all scores >= 0`. A
mechanism read out of the code but never observed at runtime says so instead:
`inferred from src/rerank.rs:88 — not measured`. This is the premise a later
session re-checks BEFORE working the bug, and an unmeasured mechanism is a
hypothesis wearing a conclusion's clothes.*

## Evidence
*One subsection per piece of evidence. Include its source (Langfuse trace id,
container logs, command output). Quote rather than summarize — copy the
relevant lines into a code fence.*

## Hypotheses tried
*Numbered list. Each entry: **Hypothesis** / **Test** (what we did to check) /
**Verdict** (confirmed | rejected | deferred) / **Evidence link** (anchor to
the Evidence subsection). Append; never delete rejected ones — they are how
future-me avoids re-walking dead ends.*

## Fix
*Plan first, implementation second. When implemented, list the commit SHA on
`master` and where the change lives (e.g. `src/rerank.rs:88-140`). If "Fix" is
just a workaround, say so explicitly and keep status `mitigated`, not `fixed`.*

*Tuning values are not fixes. A threshold, weight, or score cutoff measured
against one model on one host is an observation about that setup, not a
default — ship the probe that lets a user measure their own, or ship the knob
off by default. See `codescout:` memory `conventions` § Environment-Agnostic
Tuning for the precedent.*

## Tests added
*Regression test name + `path:line`. If the test is intentionally absent, say
why (timing-dependent, needs a live endpoint, manual-only). Empty `Tests added`
without justification means the bug isn't really closed.*

## Workarounds
*What users can do RIGHT NOW to unblock themselves while a fix lands.*

## Resume

*Concrete next action, not a goal. Bad: "investigate the rerank path". Good:
"probe localhost:30083 with the four-tier fixture in § Evidence; check whether
the adversarial middle tier separates. Run `cargo test rerank_blend` to anchor
behavior." Wipe and replace each session. `N/A` once fixed.*

*Cite paths with their prefix (`src/rerank.rs`, not bare `rerank.rs`), and
cross-repo paths with their repo (`codescout:docs/...`). If your fix moves a
file, update the Resume sections of any open bugs that cite the old path.*

## References
*Files, dashboards, related issues, external links, session log paths.*
