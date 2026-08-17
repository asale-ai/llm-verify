// SPDX-License-Identifier: Apache-2.0
//! Black-box verification for LLM API endpoints, as a library.
//!
//! The [`llm-verify`](https://crates.io/crates/llm-verify) binary is a thin CLI
//! over this crate. Everything it does is reachable here, so a caller embedding
//! the engine gets the same probes, the same order and the same verdict as the
//! published tool — which is what lets it claim the two agree.
//!
//! ```no_run
//! use llm_verify::{engine, probes::Cancel, Endpoint};
//!
//! # async fn demo() -> anyhow::Result<()> {
//! let cfg = engine::RunConfig::new(Endpoint {
//!     base_url: "https://api.anthropic.com".into(),
//!     api_key: std::env::var("ANTHROPIC_API_KEY").unwrap_or_default(),
//!     model: "claude-opus-4-5".into(),
//!     ..Default::default()
//! });
//! let report = engine::run(cfg, &Cancel::new(), &mut |_| {}).await?;
//! println!("{:?} {}", report.verdict.authenticity, report.verdict.score);
//! # Ok(())
//! # }
//! ```
//!
//! # Probing something you reach through a relay
//!
//! Half the suite asks questions *about the endpoint* — its error envelopes,
//! its response headers, the token counts it reports. Those answers describe
//! whichever hop is nearest the caller, so behind a relay they describe the
//! relay. Ask for [`probes::Selection::model_only`] and only the steps whose
//! evidence is the generated text itself will run. See [`probes::Subject`].
//!
//! ```no_run
//! # use llm_verify::{engine, probes::Cancel, Endpoint};
//! # async fn demo(endpoint: Endpoint) -> anyhow::Result<()> {
//! let cfg = engine::RunConfig::new(endpoint)
//!     .model_only()
//!     .depth(llm_verify::probes::Depth::Fast)
//!     // Chosen by the caller, recorded in the report, so a contested verdict
//!     // can be replayed probe for probe.
//!     .seed(0x5EED);
//! let report = engine::run(cfg, &Cancel::new(), &mut |_| {}).await?;
//! # Ok(())
//! # }
//! ```

#[macro_use]
pub mod i18n;

pub mod client;
pub mod engine;
pub mod pricing;
pub mod probes;
pub mod protocol;
pub mod report;
pub mod util;
pub mod verdict;

#[cfg(feature = "html")]
pub mod html;

pub use client::{Endpoint, RequestOpts};
pub use engine::{run, RunConfig};
pub use i18n::Lang;
pub use probes::{Cancel, Depth, Event, Pace, Schedule, Selection, Subject};
pub use protocol::Protocol;
pub use report::{Authenticity, Channel, ProbeResult, Report, Status, Verdict};
