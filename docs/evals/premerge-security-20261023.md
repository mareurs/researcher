# Pre-merge security review — 2026-10-23

Scope: researcher evidence-quality changes against `f862f4f`, candidate documentation,
validation artifacts, and the resolved dependency lockfile. This is a bounded review,
not a guarantee against all vulnerabilities.

- Independent review found no merge blocker in hostname validation, redirect
  provenance, evidence extraction/verification, Vertex response handling, MCP errors,
  or parent-process lifecycle. Two applied mutations were caught: removing Vertex
  finish-reason rejection and weakening hostname boundaries to substring matching.
- Updated h2, rustls, rustls-webpki, rand, and anyhow to compatible patched versions.
  `cargo audit` reports zero vulnerabilities and no unsoundness warnings. One existing
  unmaintained-dependency warning remains: fxhash through selectors/scraper
  (`RUSTSEC-2025-0057`).
- Compared candidate files, including tracked dotfiles, with configured credential
  literals and provider/private-key patterns. Redacted both valid and stale Brave
  credentials from the previously untracked SearXNG issue before committing.
  Runtime credentials and configs remain ignored.
- Ran detect-secrets without remote credential verification. Reviewed entropy
  candidates as hashes and provider metadata; template/test secrets are placeholders.
  Removed seven Vertex thoughtSignature fields from published probe records, retaining
  original hashes/lengths and recomputing published hashes. All 87 bundled records
  pass byte-length and SHA-256 checks. Original probe responses remain local.
- `cargo test --locked`: 135 passing test executions, three ignored.
  Strict Clippy and the locked release build pass.

Report files, raw opt-in traffic logging, and ADC authentication were unchanged by
the functional diff. Existing remote history is not rewritten by this merge; this
review does not attest that previously exposed credentials were rotated.
