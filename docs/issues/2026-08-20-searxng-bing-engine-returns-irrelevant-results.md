---
id: ce571bb8813c1a36
kind: bug
status: fixed
title: 'BUG: search backend chain degraded to broken SearXNG bing scraper — returns adult/marketing/off-topic sources for well-formed queries'
owners:
- marius
tags:
- search
- searxng
- brave
- config-drift
- quality
closed: 2026-10-23
opened: 2026-08-20
related:
- docs/issues/2026-07-27-researcher-rerank-score-scale-mismatch.md
severity: high
---

## Summary

`researcher` returned adult-content and parcel-tracking-site sources — plus a swathe of
topically unrelated junk (dictionary definitions, sports scores, Sony patents, Fortnite,
Sims mods) — for well-formed, specific academic queries. Root cause is a fully degraded
search backend chain: the primary backend (Brave, via a stale API key in one profile's
MCP config) is dead, and the SearXNG fallback's only non-erroring engine (`bing`) is
itself silently returning irrelevant results.

## Symptom (Effect)

A `general-purpose` subagent ran 9 `mcp__researcher__research` calls (target=topic,
intent=academic, mode=report/summary) on well-formed technical topics (bitemporal
databases, truth-maintenance systems, cache invalidation, etc.). Of the 9:

- 5/9 returned `Error: All source summaries were empty or irrelevant.`
- 4/9 "succeeded" but the subagent's own final report flagged:
  > *Instrument caveat: the `researcher` MCP returned adult-content and parcel-tracking
  > sites as "sources" on one query. Every citation above was confirmed through a
  > separate search or fetch...*

Inspecting the Langfuse `crawl` span for each trace directly showed the actual source
domains fetched per query — e.g. for topic "Knowledge graph fact validity and decay:
temporal knowledge graphs, fact expiry...":

```
cumguru.com, porn40.com, vipwank.com, www.alohatube.com,
m.usps.com, www.dhl.com, www.theupsstore.com, www.ups.com,
de.m.wikipedia.org, en.m.wikipedia.org, en.wikipedia.org, wikimediafoundation.org,
www.wikidata.org, phineasandferb.fandom.com
```

and for "Proof-carrying verification and evidence-bearing attestation...":

```
ge.xhamster.desi, xhamster.desi, www.meinsex.video,
app.proof.com, shop.sgproof.com, www.proof.com, www.microsoft.com, ...
```

Every one of the 9 traces also pulled in a generic dictionary hit
(`dictionary.cambridge.org`, `www.merriam-webster.com`, `www.dictionary.com`, or
`learnenglish.britishcouncil.org`) regardless of topic — the tell that something
upstream of the pipeline is not matching on the query at all.

## Reproduction

Commit: `e08e7039ba0c7cb852f1ee0e1154d497213d7c14`.

1. Query SearXNG directly, isolating the `bing` engine:
   ```
   curl -s "http://localhost:4000/search?q=bitemporal+data+modeling+valid+time+vs+transaction+time&format=json&engines=bing"
   ```
   Returns Michigan State Spartans football articles (`msuspartans.com`, `espn.com`,
   `ncaa.com`, ...) — nothing related to the query — with `unresponsive_engines: []`,
   i.e. SearXNG believes the engine succeeded.

2. Query the same instance without restricting engines:
   ```
   curl -s "http://localhost:4000/search?q=bitemporal+data+modeling+...+SQL:2011+temporal+database+schema+design&format=json"
   ```
   Returns Sony PlayStation patent articles — also unrelated. (Non-deterministic
   across runs — different junk each time — consistent with a broken/interstitial page
   being scraped rather than real query results.)

3. Test the Brave API key configured in the *`~/.claude-sdd` profile's global,
   user-scope* MCP entry for `researcher` (`~/.claude-sdd/.claude.json` →
   `mcpServers.researcher.env.BRAVE_API_KEY`):
   ```
   curl -s -H "X-Subscription-Token: <redacted-brave-api-key>" \
     "https://api.search.brave.com/res/v1/web/search?q=test"
   → 422 {"code":"SUBSCRIPTION_TOKEN_INVALID", ...}
   ```
   The *repo-scoped* `.mcp.json` in this project carries a **different, valid** key
   (`<redacted-brave-api-key>` → `200 OK`). The two configs have drifted.

## Environment

- Session that triggered it: `~/.claude-sdd` profile (SDD instance), working directory
  `/home/marius/work/claude/codescout` (session `a8acb1cf-81e5-4816-82f1-be2f7b273892`,
  async subagent `ac60b413d2edfd5b6`, 2026-08-20T00:23–00:33 UTC).
- `researcher` MCP is **not** registered in codescout's own `.mcp.json` — it reached the
  subagent via the `~/.claude-sdd` profile's **global/user-scope** `mcpServers.researcher`
  entry in `~/.claude-sdd/.claude.json`, which is a separate, independently-maintained
  config from this repo's project-scoped `.mcp.json` (and from the main `~/.claude`
  profile's copy). This is exactly the "check and apply to ALL THREE instances" config-drift
  risk the user's own global `~/.claude/CLAUDE.md` already warns about — it just hadn't
  bitten yet.
- SearXNG: container `searxng`, config mounted from `config/searxng/settings.yml` in this
  repo (gitignored; the committed file at that path is actually the **template**, edited
  in place rather than copied — see Root cause).
- LLM backend: Vertex `gemini-3.1-flash-lite` (irrelevant to this bug — failure is entirely
  upstream in search).

## Root cause

Two independent failures stack, and neither alone would have surfaced junk sources to the
user — both had to fail together:

**1. Primary backend (Brave, direct) is dead for this profile.** `search_with_fallback`
(`src/search/mod.rs:33`) picks the primary backend by `domain_profile` (not `intent` —
the subagent's calls set `intent="academic"` but never `domain_profile`, so routing fell
to the `None → Brave` branch, not the Exa/academic branch documented at
`src/search/mod.rs:14-20`). Brave failed with `422 SUBSCRIPTION_TOKEN_INVALID` (confirmed
by direct curl, Reproduction step 3) because the `~/.claude-sdd` profile's global MCP
config carries a stale/dead key, distinct from this repo's own `.mcp.json` key which is
live. `search_with_fallback` falls through to SearXNG on any primary-backend error.

**2. SearXNG's fallback engines are also degraded, down to one silently-broken engine.**
`config/searxng/settings.yml` enables `google` (explicitly `disabled: true`), `duckduckgo`,
`bing`, `braveapi` (SearXNG's own, separate Brave integration), and `wikipedia`:

- `braveapi`: `api_key: REPLACE_WITH_BRAVE_API_KEY` — the template placeholder was never
  replaced with a real key in the live mounted file. Confirmed via `docker logs searxng`:
  `HTTPStatusError: Client error '422 Unprocessable Entity' ... api.search.brave.com`.
- `duckduckgo`: CAPTCHA-blocked. Confirmed via `docker logs searxng`:
  `SearxEngineCaptchaException: CAPTCHA (us-en) (suspended_time=0)`.
- `bing`: **returns HTTP 200 with `unresponsive_engines: []` — SearXNG believes it
  succeeded — but the actual results are unrelated to the query** (Reproduction steps
  1–2). This is consistent with Bing's scraper-detection serving a generic/interstitial
  or region-mismatched page instead of a real SERP, which SearXNG's `bing.py` parser then
  scrapes as if it were legitimate results, with no relevance signal to catch it.
- `wikipedia`: apparently working (explains the recurring `en.wikipedia.org` hit per
  trace) but contributes little on its own.

With Brave dead and three of SearXNG's five engines non-functional, essentially all
"search results" this profile's `researcher` calls received during this run came from
the broken `bing` engine, which is why every one of the 9 traces shows the same
signature: a stray dictionary-site hit (bing's junk page apparently always includes one)
plus topically-unrelated filler that happened to include, this time, adult and
parcel-tracking spam.

**3. Nothing downstream catches "these sources have nothing to do with the query."**
`quality.rs`'s filter is word-count/text-density only (no topical relevance check), and
the reranker that could plausibly catch this is itself mid-migration and effectively
inert or miscalibrated (`docs/issues/2026-07-27-researcher-rerank-score-scale-mismatch.md`).
So junk sources that pass the word-count bar flow straight through to summarization —
where the LLM judge does catch some of it (5/9 queries in this run correctly came back
`Error: All source summaries were empty or irrelevant.`), but the other 4/9 slipped a
subset of junk citations past the summarizer into an otherwise-real report.

*Measured 2026-08-20: all `curl` reproductions above, plus `docker logs searxng --tail 400`
for the engine-level errors, plus the Langfuse `crawl` span outputs for trace ids
`822d31d0-4f32-4479-b408-433f8f25b49f` and `0553be38-5750-48ec-8375-a6c9329656ef`
(this project's Langfuse instance, `http://localhost:3000`).*

## Evidence

### Langfuse crawl-span domains, per trace (2026-08-20T00:23–00:28 UTC)

Pulled via `GET /api/public/traces/{id}`, `observations[].name == "crawl"`. Full domain
lists for all 9 traces from this run are in the session investigation; two representative
ones:

```
trace 822d31d0 "Knowledge graph fact validity and decay...":
  cumguru.com, porn40.com, vipwank.com, www.alohatube.com, m.usps.com, www.dhl.com,
  www.theupsstore.com, www.ups.com, de.m.wikipedia.org, en.m.wikipedia.org,
  en.wikipedia.org, wikimediafoundation.org, www.wikidata.org, phineasandferb.fandom.com

trace 0553be38 "Proof-carrying verification and evidence-bearing attestation...":
  app.proof.com, en.wikipedia.org, ge.xhamster.desi, myaccount.google.com, remote.co,
  remote.com, remotedesktop.google.com, shop.sgproof.com, www.comptia.org,
  www.meinsex.video, www.microsoft.com, www.proof.com, xhamster.desi
```

### SearXNG engine errors, `docker logs searxng --tail 60`

```
httpx.HTTPStatusError: Client error '422 Unprocessable Entity' for url
'https://api.search.brave.com/res/v1/web/search?q=spaced+repetition+algorithms...'
2026-08-20 01:05:00,770 ERROR:searx.engines.duckduckgo: CAPTCHA (us-en)
searx.exceptions.SearxEngineCaptchaException: CAPTCHA (us-en) (suspended_time=0)
2026-08-20 01:05:01,087 ERROR:searx.engines.braveapi: requests exception ... :
Client error '422 Unprocessable Entity' for url
'https://api.search.brave.com/res/v1/web/search?q=bitemporal+data+modeling...'
```

### `bing` engine isolated, direct curl

```
$ curl -s "http://localhost:4000/search?q=bitemporal+data+modeling+valid+time+vs+transaction+time&format=json&engines=bing"
→ number_of_results: 2890, unresponsive_engines: []
→ results[0..9] are all Michigan State Spartans football articles
  (msuspartans.com, si.com, espn.com, ncaa.com, huskers.com, apnews.com, ...)
```

### Brave key drift between profiles

```
repo .mcp.json key   (<redacted-brave-api-key>) → 200 OK
sdd profile key       (<redacted-brave-api-key>) → 422 SUBSCRIPTION_TOKEN_INVALID
```

### `config/searxng/settings.yml` — braveapi placeholder never replaced

```yaml
  - name: braveapi
    engine: braveapi
    shortcut: brapi
    api_key: REPLACE_WITH_BRAVE_API_KEY
    disabled: false
```

## Hypotheses tried

1. **Hypothesis:** the vague user instruction ("find out what industry/research says
   about an elegant solution to our problem") was passed to the search tool literally,
   unrefined. **Test:** read the actual subagent prompt and its 9 `research()` call
   inputs from the session transcript. **Verdict:** rejected — every query was a
   specific, well-formed academic search string; the orchestrating session had already
   built a proper research brief. **Evidence link:** § Reproduction / session transcript
   at `~/.claude-sdd/projects/-home-marius-work-claude-codescout/a8acb1cf-.../subagents/agent-ac60b413d2edfd5b6.jsonl`.
2. **Hypothesis:** the known rerank-scale-mismatch bug (`2026-07-27-...`) is directly
   responsible — junk sources rank above good ones. **Test:** checked whether any
   content-relevance filtering exists anywhere upstream of rerank. **Verdict:** rejected
   as *sole* cause, but noted as a contributing factor with nothing to rerank *from* —
   the sources fetched were themselves off-topic, not merely mis-ordered.
   **Evidence link:** § Root cause point 3.
3. **Hypothesis:** the query never reached `researcher`'s pipeline at all (e.g. the
   subagent used `WebSearch` instead of the MCP tool, or the tool wasn't bound).
   **Test:** grepped the subagent transcript for tool names used. **Verdict:** rejected —
   `mcp__researcher__research` was called 9 times with real results; `WebSearch` was
   also used separately for named-paper follow-ups per its instructions, unrelated to
   this bug. **Evidence link:** § Reproduction.
4. **Hypothesis:** SearXNG's `bing` engine is silently returning irrelevant content.
   **Test:** isolated `engines=bing` directly against the live SearXNG instance.
   **Verdict:** confirmed. **Evidence link:** § Evidence / "`bing` engine isolated".
5. **Hypothesis:** the primary Brave backend is unreachable for this profile, forcing
   the SearXNG fallback path. **Test:** curled the Brave API directly with the exact key
   from the `~/.claude-sdd` profile's MCP config. **Verdict:** confirmed — key is dead;
   repo's own key is fine. **Evidence link:** § Evidence / "Brave key drift between
   profiles".

## Fix
### Resolution measured 2026-10-23

The three existing Claude profiles now already share the same working Brave credential and healthy TEI port30083; they were checked without exposing values. The active Codex profile and ignored project `.mcp.json` had reranker48083 drift and were corrected to30083 with absolute score filtering disabled. The SearXNG Brave credential differed from that working credential; it was reconciled in the ignored local settings and SearXNG restarted. A retained identical Tokio query changed from0 results (Brave HTTP error, DDG CAPTCHA) to11 results, all via Brave, led by the official Tokio select documentation. Bing remains disabled. DDG CAPTCHA remains an external engine limitation; the functioning Brave engine provides the fallback.

Code now enforces parsed hostname boundaries on every backend, preserves explicitly requested Romanian domains, retains final redirected source URLs, and rejects redirects outside requested domains. Source relevance is checked against actual text before publication, and unsupported generated reports are repaired or replaced with attributed evidence. Regression fixtures and applied mutation review cover these paths. Private credentials remain outside tracked files. Historical investigation below is retained as dated evidence.


Not yet implemented. Three independent things need fixing, in priority order:

1. **Rotate/replace the dead Brave API key in `~/.claude-sdd/.claude.json`**
   (`mcpServers.researcher.env.BRAVE_API_KEY`), and reconcile it with the other two
   profiles' copies (`~/.claude/`, `~/.claude-kat/`) plus this repo's own `.mcp.json`, per
   the user's standing instruction to keep all three profiles' MCP config in sync. This
   alone would have prevented SearXNG ever being reached for this run.
2. **Fill in the real Brave key in `config/searxng/settings.yml`'s `braveapi` engine**
   (currently the literal placeholder `REPLACE_WITH_BRAVE_API_KEY`), or disable that
   engine entry if it's not meant to be live — a placeholder that silently 422s on every
   query is worse than an explicitly disabled engine.
3. **Done 2026-08-20: `bing` disabled in `config/searxng/settings.yml`**, mirroring how
   `google` was already disabled for the same class of failure. Verified post-restart:
   the same query now returns `number_of_results: 0` with `unresponsive_engines:
   [["braveapi","HTTP error"],["duckduckgo","CAPTCHA"]]` instead of silently serving
   off-topic junk — fails loud instead of fails wrong. **New consequence: SearXNG now
   has zero working engines** (google + bing disabled, duckduckgo CAPTCHA-blocked,
   braveapi still has the placeholder key from item 2). Until item 1 or item 2 lands,
   SearXNG is a pure pass-through of whatever `researcher`'s primary backend returns —
   fine only as long as that backend's own key (item 1) is valid.
   Still open: add a cheap topical-relevance sanity check in the pipeline (e.g.
   flag/drop a source whose scraped text shares no meaningful terms with the query) as
   defense-in-depth, independent of the reranker fix tracked in
   `docs/issues/2026-07-27-researcher-rerank-score-scale-mismatch.md`.

## Tests added

`N/A` — no automated regression test added yet. The failure is in external service
health/config (a dead API key, an upstream scraper silently breaking), not in this
repo's own logic, so there's nothing here a unit test would catch. A live-fire smoke
test (real search call, assert returned domains share vocabulary with the query) would
have caught this and is worth adding once the immediate config fixes land — tracked in
§ Resume.

## Workarounds

- Set a real `domain_profile` (not just `intent`) on academic queries so `Exa` is tried
  first instead of falling to the (currently broken) Brave→SearXNG chain —
  `domain_profile="academic"` per `src/search/mod.rs:14-20`.
- Until the sdd-profile Brave key is rotated, prefer running research from this repo's
  own project-scoped `.mcp.json` (which carries a live key) rather than via a subagent
  spawned from a project relying on the global `~/.claude-sdd` MCP registration.
- Manually review source domains in any report produced while this is open before
  trusting citations.

## Resume

Rotate the Brave key in `~/.claude-sdd/.claude.json` (`mcpServers.researcher.env.BRAVE_API_KEY`)
and confirm with `curl -H "X-Subscription-Token: <key>" https://api.search.brave.com/res/v1/web/search?q=test`
→ expect `200`. Then reconcile the same key across `~/.claude/`, `~/.claude-kat/`, and
this repo's `.mcp.json` so they don't drift again. Separately, decide on `bing` in
`config/searxng/settings.yml` (disable vs. add relevance guard) and fill in or remove the
`braveapi` engine's placeholder key. Re-run one of the 9 failing topics from this bug's
session transcript through `research()` directly and confirm the crawl-span domains are
on-topic.

## References

- Session transcript (parent): `~/.claude-sdd/projects/-home-marius-work-claude-codescout/a8acb1cf-81e5-4816-82f1-be2f7b273892.jsonl`
- Subagent transcript: `~/.claude-sdd/projects/-home-marius-work-claude-codescout/a8acb1cf-81e5-4816-82f1-be2f7b273892/subagents/agent-ac60b413d2edfd5b6.jsonl`
- Langfuse traces (this project's instance, `http://localhost:3000`): `822d31d0-4f32-4479-b408-433f8f25b49f`, `0553be38-5750-48ec-8375-a6c9329656ef`, plus 7 sibling traces from the same run (00:23:26–00:27:43 UTC, 2026-08-20).
- Related: `docs/issues/2026-07-27-researcher-rerank-score-scale-mismatch.md` (reranker miscalibration — contributing factor, not root cause, of this bug).
- `src/search/mod.rs:13-53` (`search_with_fallback` routing/fallback logic).
- `config/searxng/settings.yml` (live, gitignored — engine enable/disable + keys).
