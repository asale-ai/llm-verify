// SPDX-License-Identifier: Apache-2.0
//! llm-verify — black-box verification for LLM API endpoints.
//!
//! A CLI over the [`llm_verify`] crate. Argument parsing, the terminal display
//! and the report files live here; everything that decides anything lives in
//! the library, so this binary and an embedded caller cannot drift apart.

mod term;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use llm_verify::client::Endpoint;
use llm_verify::i18n;
use llm_verify::probes::{Cancel, Depth, Event, Selection};
use llm_verify::protocol::Protocol;
use llm_verify::report::Report;
use llm_verify::{engine, html, t, ts, util};
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Parser)]
#[command(
    name = "llm-verify",
    version,
    about = "Verify the LLM endpoint you are actually using: model authenticity, \
              billing inflation, relay provenance, performance and silent downgrades",
    long_about = None,
    disable_help_subcommand = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    run: RunArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Verify an endpoint (the default command)
    Run(RunArgs),
}

#[derive(clap::Args, Clone, Default)]
struct RunArgs {
    /// Endpoint base URL, e.g. https://api.anthropic.com
    #[arg(long)]
    base_url: Option<String>,

    /// API key
    #[arg(long)]
    api_key: Option<String>,

    /// Model ID to verify
    #[arg(long)]
    model: Option<String>,

    /// Protocol: anthropic or openai. Inferred from the URL and model if unset
    #[arg(long)]
    protocol: Option<String>,

    /// The model the vendor claims to serve. Defaults to --model
    #[arg(long)]
    claimed_model: Option<String>,

    /// Report language: en / zh. Defaults to the system locale, then English.
    #[arg(long)]
    lang: Option<String>,

    /// Probe depth: fast / balanced / forensic
    #[arg(long, default_value = "balanced")]
    depth: String,

    /// Run only the probes whose evidence is the generated text.
    ///
    /// Use when the endpoint is a relay you already know about and what you
    /// want to know is what is behind it: the contract, header and metering
    /// probes would all be describing the relay.
    #[arg(long)]
    model_only: bool,

    /// The quickest run that still reaches a verdict: --model-only cut down to
    /// the probes a conclusion rests on, at fast depth, several at a time.
    ///
    /// Nine requests instead of twenty-one, and in about four round trips
    /// rather than nine. What it gives up is the corroborating and merely
    /// descriptive identity probes, and the three contract checks that catch an
    /// endpoint reconstructed from a web session — so reach for it when you
    /// already know what kind of thing is on the far end and want to know
    /// whether the model behind it is the one advertised.
    #[arg(long)]
    turbo: bool,

    /// Requests to keep in flight. 1 is sequential, which is the default.
    ///
    /// Only worth raising against an endpoint you know will answer that many at
    /// once: overrun its concurrency and the run measures the queue rather than
    /// the model. Latency probes always run alone regardless.
    #[arg(long, default_value_t = 1)]
    concurrency: usize,

    /// Comma-separated probe ids to run; everything else is skipped.
    /// A group name (contract, identity, billing, …) selects the whole group.
    #[arg(long, value_delimiter = ',')]
    only: Vec<String>,

    /// Comma-separated probe ids or group names to skip
    #[arg(long, value_delimiter = ',')]
    skip: Vec<String>,

    /// Seed the probe payloads, so the run can be reproduced exactly.
    ///
    /// Leave unset for real use: an endpoint that can predict the payloads can
    /// pre-cache the answers.
    #[arg(long)]
    seed: Option<u64>,

    /// HTML report path. Pass a directory to auto-name the file
    #[arg(long, short = 'o')]
    out: Option<PathBuf>,

    /// Also write a JSON report
    #[arg(long)]
    json: Option<PathBuf>,

    /// Per-request timeout, in seconds
    #[arg(long, default_value_t = 120)]
    timeout: u64,

    /// Read configuration from this .env file
    #[arg(long, default_value = ".env")]
    env_file: PathBuf,

    /// Suppress per-probe progress output
    #[arg(long)]
    quiet: bool,

    /// Disable coloured output
    #[arg(long)]
    no_color: bool,

    /// Do not open the report in a browser
    #[arg(long)]
    no_open: bool,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Run(args)) => run_blocking(args),
        None => run_blocking(cli.run),
    }
}

fn run_blocking(args: RunArgs) -> Result<()> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .context("failed to start the async runtime")?;
    let code = rt.block_on(run(args))?;
    std::process::exit(code);
}

/// Minimal `.env` reader. A dotenv crate would be another dependency for
/// twenty lines; this deliberately does not export into the process
/// environment, so a key read here cannot leak into a child process.
fn load_env_file(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let v = v.trim().trim_matches('"').trim_matches('\'');
            Some((k.trim().to_string(), v.to_string()))
        })
        .collect()
}

fn pick(pairs: &[(String, String)], keys: &[&str]) -> Option<String> {
    for k in keys {
        if let Some((_, v)) = pairs.iter().find(|(pk, _)| pk == k) {
            if !v.trim().is_empty() {
                return Some(v.clone());
            }
        }
        if let Ok(v) = std::env::var(k) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// Guess the protocol from the base URL and model name when not told.
fn infer_protocol(base_url: &str, model: &str) -> Protocol {
    let u = base_url.to_ascii_lowercase();
    let m = model.to_ascii_lowercase();
    if u.contains("anthropic") || u.contains("/v1/messages") {
        return Protocol::Anthropic;
    }
    if u.contains("openai.com") || u.contains("openrouter") || u.contains("chat/completions") {
        return Protocol::OpenAI;
    }
    // A Claude model name on a neutral host is more likely to be served over
    // the Messages API than over chat/completions.
    if m.contains("claude") {
        return Protocol::Anthropic;
    }
    Protocol::OpenAI
}

async fn run(args: RunArgs) -> Result<i32> {
    let env_pairs = load_env_file(&args.env_file);
    // Errors below are user-facing, so resolve the language before them.
    let lang_early = i18n::Lang::from_env(args.lang.as_deref());

    let base_url = args
        .base_url
        .clone()
        .or_else(|| {
            pick(
                &env_pairs,
                &[
                    "LLM_VERIFY_BASE_URL",
                    "VERIFY_BASE_URL",
                    "ANTHROPIC_BASE_URL",
                    "OPENAI_BASE_URL",
                    "OPENAI_API_BASE_URL",
                ],
            )
        })
        .ok_or_else(|| {
            anyhow!(t!(
                lang_early,
                "missing --base-url (or set LLM_VERIFY_BASE_URL, or put it in .env)",
                "缺少 --base-url（也可用 LLM_VERIFY_BASE_URL 环境变量或 .env 提供）"
            ))
        })?;

    let api_key = args
        .api_key
        .clone()
        .or_else(|| {
            pick(
                &env_pairs,
                &[
                    "LLM_VERIFY_API_KEY",
                    "VERIFY_API_KEY",
                    "ANTHROPIC_API_KEY",
                    "OPENAI_API_KEY",
                ],
            )
        })
        .unwrap_or_default();

    let model = args
        .model
        .clone()
        .or_else(|| pick(&env_pairs, &["LLM_VERIFY_MODEL", "VERIFY_MODEL"]))
        .ok_or_else(|| {
            anyhow!(t!(
                lang_early,
                "missing --model (or set LLM_VERIFY_MODEL, or put it in .env)",
                "缺少 --model（也可用 LLM_VERIFY_MODEL 环境变量或 .env 提供）"
            ))
        })?;

    let protocol = match args
        .protocol
        .clone()
        .or_else(|| pick(&env_pairs, &["LLM_VERIFY_PROTOCOL", "VERIFY_PROTOCOL"]))
    {
        Some(p) => Protocol::parse(&p).ok_or_else(|| {
            anyhow!(t!(
                lang_early,
                "unrecognised protocol {p}; expected anthropic or openai",
                "无法识别的协议 {p}，可选：anthropic / openai"
            ))
        })?,
        None => infer_protocol(&base_url, &model),
    };

    let lang = lang_early;
    let depth = Depth::parse(&args.depth).ok_or_else(|| {
        anyhow!(t!(
            lang,
            "unrecognised depth {}; expected fast, balanced or forensic",
            "无法识别的深度 {}，可选：fast / balanced / forensic",
            args.depth
        ))
    })?;
    // `--turbo` implies fast depth, but an explicit `--depth` still wins: the
    // preset is a starting point, not a mode that locks the other flags out.
    // Resolved here rather than at the call site so the banner names the depth
    // the run will actually use.
    let depth = match (args.turbo, depth) {
        (true, Depth::Balanced) => Depth::Fast,
        (_, d) => d,
    };

    let claimed_model = args.claimed_model.clone().unwrap_or_else(|| model.clone());

    let endpoint = Endpoint {
        base_url: base_url.clone(),
        api_key,
        protocol,
        model: model.clone(),
        anthropic_version: pick(&env_pairs, &["ANTHROPIC_VERSION"])
            .unwrap_or_else(|| "2023-06-01".to_string()),
        timeout: Duration::from_secs(args.timeout),
        headers: Vec::new(),
    };

    let use_color = !args.no_color && std::env::var("NO_COLOR").is_err();

    if !args.quiet {
        term::banner(&endpoint, depth, &claimed_model, lang, use_color);
    }

    let selection = if args.turbo {
        Selection::turbo()
    } else if args.model_only {
        Selection::model_only()
    } else {
        Selection {
            only: args.only.clone(),
            skip: args.skip.clone(),
            ..Default::default()
        }
    };

    // Same rule as the depth override above: the preset picks a starting point
    // and an explicit `--concurrency` overrides it.
    let overlap = match (args.turbo, args.concurrency) {
        (true, 1) => 3,
        (_, n) => n,
    };
    let mut cfg = engine::RunConfig::new(endpoint)
        .depth(depth)
        .lang(lang)
        .claimed_model(claimed_model.clone())
        .concurrency(overlap)
        // Steps are uneven, so bounding them does not bound the traffic. This
        // does, and it is what the endpoint actually feels.
        .max_in_flight(overlap);
    cfg.selection = selection;
    cfg.seed = args.seed;

    let mut on_event = |ev: Event<'_>| {
        if args.quiet {
            return;
        }
        if let Event::Finished {
            result,
            done,
            total,
        } = ev
        {
            term::progress_line(result, done, total, lang, use_color);
        }
    };
    let rep = engine::run(cfg, &Cancel::new(), &mut on_event).await?;

    if !args.quiet {
        term::summary(&rep, use_color);
    }

    if let Some(path) = &args.json {
        let json = serde_json::to_string_pretty(&rep)?;
        let name = default_report_name(&rep).replace(".html", ".json");
        let written = write_out(path, json.as_bytes(), &name)?;
        if !args.quiet {
            println!(
                "  {} : {}",
                ts!(lang, "JSON report", "JSON 报告"),
                written.display()
            );
        }
    }

    // HTML is the primary artefact, so it is written unless explicitly
    // redirected: a run that only prints to a terminal loses the evidence.
    let html_path = args
        .out
        .clone()
        .unwrap_or_else(|| PathBuf::from(default_report_name(&rep)));
    let html = html::render(&rep);
    let written = write_out(&html_path, html.as_bytes(), &default_report_name(&rep))?;
    if !args.quiet {
        println!(
            "  {} : {}",
            ts!(lang, "HTML report", "HTML 报告"),
            written.display()
        );
        if !args.no_open {
            open_in_browser(&written);
        }
    }

    Ok(rep.exit_code())
}

fn default_report_name(rep: &Report) -> String {
    let host = rep
        .host
        .replace([':', '/', '.'], "-")
        .trim_matches('-')
        .to_string();
    format!("llm-verify-{host}-{}.html", util::file_stamp())
}

fn resolve_path(path: &Path, default_name: &str) -> PathBuf {
    if path.is_dir() {
        path.join(default_name)
    } else {
        path.to_path_buf()
    }
}

fn write_out(path: &Path, bytes: &[u8], default_name: &str) -> Result<PathBuf> {
    let target = resolve_path(path, default_name);
    if let Some(parent) = target.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("could not create directory {}", parent.display()))?;
        }
    }
    std::fs::write(&target, bytes)
        .with_context(|| format!("could not write {}", target.display()))?;
    std::fs::canonicalize(&target).or(Ok(target))
}

/// Best-effort. A failure to open a browser must never fail the run — the
/// file is already written and its path was printed.
fn open_in_browser(path: &std::path::Path) {
    let cmd = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(cmd)
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_inference_prefers_explicit_host_signals() {
        assert_eq!(
            infer_protocol("https://api.anthropic.com", "claude-opus-4-5"),
            Protocol::Anthropic
        );
        assert_eq!(
            infer_protocol("https://api.openai.com/v1", "gpt-4o"),
            Protocol::OpenAI
        );
        assert_eq!(
            infer_protocol("https://openrouter.ai/api/v1", "anthropic/claude-opus-4-5"),
            Protocol::OpenAI,
            "OpenRouter speaks chat/completions even for Claude models"
        );
        // Neutral host: fall back to the model name.
        assert_eq!(
            infer_protocol("https://relay.example/v1", "claude-sonnet-4-5"),
            Protocol::Anthropic
        );
        assert_eq!(
            infer_protocol("https://relay.example/v1", "some-model"),
            Protocol::OpenAI
        );
    }

    #[test]
    fn env_file_parsing_handles_comments_and_quotes() {
        let dir = std::env::temp_dir().join(format!("llmv-test-{}", util::now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join(".env");
        std::fs::write(
            &f,
            "# a comment\n\nFOO=bar\nQUOTED=\"has spaces\"\nSINGLE='x'\nEMPTY=\nBAD_LINE\n",
        )
        .unwrap();
        let pairs = load_env_file(&f);
        assert_eq!(pick(&pairs, &["FOO"]).as_deref(), Some("bar"));
        assert_eq!(pick(&pairs, &["QUOTED"]).as_deref(), Some("has spaces"));
        assert_eq!(pick(&pairs, &["SINGLE"]).as_deref(), Some("x"));
        // An empty value must fall through so the next source can supply it.
        assert_eq!(pick(&pairs, &["EMPTY"]), None);
        assert_eq!(pick(&pairs, &["BAD_LINE"]), None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_env_file_is_not_an_error() {
        assert!(load_env_file(&PathBuf::from("/nonexistent/.env")).is_empty());
    }

    #[test]
    fn pick_prefers_the_first_matching_key() {
        let pairs = vec![
            ("SECOND".to_string(), "b".to_string()),
            ("FIRST".to_string(), "a".to_string()),
        ];
        assert_eq!(pick(&pairs, &["FIRST", "SECOND"]).as_deref(), Some("a"));
        assert_eq!(pick(&pairs, &["MISSING", "SECOND"]).as_deref(), Some("b"));
    }

    #[test]
    fn report_filename_is_filesystem_safe() {
        let name = default_report_name(&test_report("api.example.com:8443"));
        assert!(!name.contains(':'));
        assert!(!name.contains('/'));
        assert!(name.starts_with("llm-verify-api-example-com-8443-"));
        assert!(name.ends_with(".html"));
    }

    fn test_report(host: &str) -> Report {
        Report {
            schema_version: llm_verify::report::schema_version(),
            tool_version: "0".into(),
            lang: i18n::Lang::En,
            started_at: String::new(),
            finished_at: String::new(),
            duration_ms: 0,
            host: host.into(),
            base_url: String::new(),
            protocol: Protocol::Anthropic,
            model: "m".into(),
            claimed_model: "m".into(),
            depth: "fast".into(),
            seed: 0,
            steps: vec![],
            request_count: 0,
            results: vec![],
            verdict: llm_verify::report::Verdict {
                authenticity: llm_verify::report::Authenticity::Inconclusive,
                channel: llm_verify::report::Channel::Unknown,
                score: 0.0,
                confidence: 0.0,
                hard_gate_hits: vec![],
                signals: vec![],
                trace: vec![],
                group_scores: Default::default(),
                coverage_gap: 0.0,
            },
            identity: Default::default(),
            billing: Default::default(),
            channel: Default::default(),
            perf: Default::default(),
            skipped: vec![],
        }
    }
}
