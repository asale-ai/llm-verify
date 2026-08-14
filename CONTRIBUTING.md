# Contributing

**English** · [简体中文](CONTRIBUTING.zh-CN.md)

## Build

Rust 1.82 or newer.

```bash
cargo build            # debug
cargo build --release  # release, ~2 MB stripped binary
cargo test             # unit tests, no network access required
cargo clippy -- -D warnings
cargo fmt --check
```

The release profile optimises for size (`opt-level = "z"`, fat LTO, `panic = "abort"`, stripped). This tool is I/O bound, so size matters more than codegen quality.

## Dependency policy

The binary ships to five platforms and gets installed by a one-line `curl | sh`, so every dependency is weighed against its size. Current direct dependencies: `clap`, `reqwest`, `tokio`, `futures-util`, `serde`, `serde_json`, `anyhow`.

Deliberately avoided, with the reasoning recorded in `src/util.rs`:

- **chrono / time** — `iso8601_utc()` is 20 lines of civil-from-days arithmetic.
- **rand** — `Rng` is a 15-line xorshift64\*. Probe payloads only need to be unpredictable to a provider, not cryptographically random.
- **tiktoken** — the rank tables cost several megabytes. Where an endpoint offers an authoritative `count_tokens` route we use it; elsewhere a calibrated heuristic covers the signal we actually need, and every conclusion drawn from it is labelled an estimate.
- **a colour crate** — `src/term.rs` writes the eight ANSI codes it uses.
- **a template engine** — `src/html.rs` builds the report with `write!` against a `const CSS`.

`reqwest` uses `rustls-tls`, not OpenSSL, so cross-compilation needs no system libraries.

## Bilingual output

Every user-facing string must exist in both languages, written at the point it
is used with both halves side by side:

```rust
p.pass(t!(l, "Endpoint reachable, {}ms", "端点可达，{}ms", raw.duration_ms))
```

- `t!` yields `String`, `ts!` yields `&'static str`. Both halves receive the
  same arguments, so a mismatched placeholder count is a compile error.
- `t!` goes through `format!` even with no extra arguments, because the
  messages lean on inline captures like `{host}`. A literal brace in a message
  must therefore be written `{{`.
- Classification results carry a **stable key**, never a display name. The
  verdict layer routes on those keys, so a translation can never change a
  verdict — see `probes/channel.rs`.
- `i18n::coverage_tests` enforces the pairing at the source level. It caught a
  real incident: `contract.rs` once shipped Chinese-only, and an English run
  silently printed Chinese for a third of its probes.

## Layout

```
src/
  main.rs        CLI, env/.env resolution, output writing
  i18n.rs        Lang enum, locale detection, the t! / ts! macros
  protocol.rs    OpenAI + Anthropic wire formats
  client.rs      HTTP transport and SSE parsing
  probes/
    mod.rs       registry, run order, shared Ctx
    contract.rs  protocol contract (12)
    stream.rs    SSE behaviour (3)
    billing.rs   metering and billing audit (7)
    identity.rs  model identity and capability tier (8)
    consistency.rs cross-request consistency (3)
    perf.rs      TTFT / latency / throughput / jitter (4)
    channel.rs   relay provenance (3)
  verdict.rs     scoring, hard gates, two-axis decision
  report.rs      the data model every probe writes into
  html.rs        self-contained HTML report
  term.rs        terminal output
  pricing.rs     built-in list prices
  util.rs        time, PRNG, token estimation, formatting
skills/
  llm-verify/SKILL.md   the agent skill, installed with `npx skills add`
```

The skill is a plain checked-in file, not something the binary writes. Edit
`skills/llm-verify/SKILL.md` directly; `npx skills add asale-ai/llm-verify`
picks it up from the repository.

Probes run sequentially and share a `Ctx` through `RefCell`. Contract probes run first on purpose: if the channel rewrites requests, later fingerprint results are unreliable and the verdict layer needs to know that before reading them.

## Adding a probe

1. Write a function returning `ProbeResult` in the right `src/probes/*.rs`.
2. Register it in `run_all` in `src/probes/mod.rs`.
3. Bump `PROBE_COUNT`.
4. If the verdict layer should react to it, read it by ID in `src/verdict.rs`.

### Rules a probe must follow

These exist because the project's worst failure mode is a false accusation against an honest provider, not a miss.

- **Prefer `Skip` over `Fail` when the endpoint simply lacks a feature.** A relay without `/models` is not fraudulent.
- **Never assert on a ratio computed over small numbers.** A constant per-request overhead reads as a huge ratio on a tiny prompt: measured against a real gateway, a 6-token prompt showed 1.75x while a 458-token prompt on the *same endpoint* showed 1.01x. Gate on an absolute delta as well as a ratio.
- **Never ask the model to do something adversarial and read refusal as middleware failure.** An earlier `system_adherence` probe told the model to ignore the user's question; stronger models correctly refused, and the probe misread that as a dropped system prompt. Carry a unique token *in* the system prompt and ask for it back instead.
- **Direction matters.** Measuring capability *above* the claim is over-delivery, not fraud, and must never trip a gate.
- **Generate payloads fresh each run.** Fixed questions can be pre-cached by a provider. Use `ctx.rng`.
- **Mark evidence-gathering probes `.neutral()`** so they inform the verdict without moving the score.

## Testing

Unit tests are inline `#[cfg(test)]` modules and must not touch the network. The interesting ones assert on the guard rails: `verdict::tests::family_mismatch_on_a_weak_signal_degrades_to_ambiguous`, `narrow_tier_margin_withdraws_the_severity_claim`, `reverse_weight_tests::weak_signals_alone_stay_below_the_threshold`, `i18n::coverage_tests::every_chinese_literal_has_an_english_partner`.

For end-to-end checks against a real endpoint, put credentials in `.env` (git-ignored) and run against a known-good provider first, so a change in the tool is distinguishable from a change in the endpoint.

## Release

`publish.sh` runs the whole flow unattended:

```bash
./publish.sh "commit message"          # patch bump
./publish.sh -m minor "commit message" # minor bump
./publish.sh --dry-run "message"       # show what would happen
```

It bumps `Cargo.toml`, commits, pushes, tags, and pushes the tag. The tag triggers `.github/workflows/release.yml`, which cross-compiles all five targets, packages them with `SHA256SUMS`, and creates the GitHub Release.

Credentials come from `.env` and are never written into the repository.
