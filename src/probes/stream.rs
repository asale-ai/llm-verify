// SPDX-License-Identifier: Apache-2.0
//! Streaming probes.
//!
//! In field measurements of low-scoring relays, broken SSE — empty bodies,
//! keep-alive frames with no content, missing terminators — was the single
//! largest failure category, well ahead of anything about model quality.

use super::{Ctx, PerfSample};
use crate::protocol::{ChatRequest, Protocol};
use crate::report::{Group, ProbeResult};
use crate::util::now_ms;

const G: Group = Group::Stream;

fn stream_req(ctx: &Ctx) -> ChatRequest {
    ChatRequest::new(
        &ctx.client.endpoint.model,
        "List the numbers 1 through 12, separated by commas. Nothing else.",
    )
    .max_tokens(96)
    .temperature(0.0)
}

pub async fn sse_format(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "sse_format",
        ts!(l, "SSE event sequence", "SSE 事件序列"),
        G,
    )
    .weight(2);
    let t0 = now_ms();
    let proto = ctx.client.endpoint.protocol;

    let s = match ctx.client.stream(&stream_req(ctx)).await {
        Ok(s) => s,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };

    // The stream carries the only TTFT measurement we can trust, so record it
    // regardless of whether the format checks pass.
    ctx.add_perf(PerfSample {
        probe: "sse_format".into(),
        ttft_ms: s.ttft_ms,
        latency_ms: s.total_ms,
        output_tokens: s
            .usage
            .as_ref()
            .map(|u| u.output_tokens)
            .filter(|n| *n > 0)
            .unwrap_or_else(|| crate::util::estimate_tokens(&s.text)),
    });

    let names = s.event_names();
    let took = (now_ms() - t0) as u64;
    let mut p = p
        .metric("event_count", names.len())
        .metric("bytes", s.bytes)
        .metric("content_type", s.content_type.clone())
        .metric("saw_done", s.saw_done_sentinel)
        .metric("ttft_ms", s.ttft_ms.unwrap_or(0))
        .evidence(crate::util::truncate(&s.text, 200));

    let mut problems: Vec<String> = Vec::new();

    if !s.content_type.contains("event-stream") {
        problems.push(t!(
            l,
            "Content-Type is {}, not text/event-stream",
            "Content-Type 是 {}，不是 text/event-stream",
            if s.content_type.is_empty() {
                t!(l, "(empty)", "(空)")
            } else {
                s.content_type.clone()
            }
        ));
    }
    if names.is_empty() {
        problems.push(t!(
            l,
            "No SSE events received at all",
            "没有收到任何 SSE 事件"
        ));
    }

    match proto {
        Protocol::Anthropic => {
            // The documented lifecycle. A relay that synthesises the stream
            // itself usually drops the bookend events.
            for required in ["message_start", "content_block_delta", "message_stop"] {
                if !names.iter().any(|n| n == required) {
                    problems.push(t!(
                        l,
                        "Missing the {required} event",
                        "缺少 {required} 事件"
                    ));
                }
            }
            if let (Some(first), Some(last)) = (names.first(), names.last()) {
                if first != "message_start" {
                    p = p.finding(t!(
                        l,
                        "First event was {first}; the spec requires message_start",
                        "首个事件是 {first}，规范应为 message_start"
                    ));
                }
                if last != "message_stop" {
                    p = p.finding(t!(
                        l,
                        "Last event was {last}; the spec requires message_stop",
                        "末个事件是 {last}，规范应为 message_stop"
                    ));
                }
            }
        }
        Protocol::OpenAI => {
            if !s.saw_done_sentinel {
                problems.push(t!(
                    l,
                    "Missing the data: [DONE] terminator",
                    "缺少 data: [DONE] 终止哨兵"
                ));
            }
        }
    }

    if let Some(err) = &s.error {
        problems.push(t!(l, "Error inside the stream: {err}", "流中报错：{err}"));
    }

    for pr in &problems {
        p = p.finding(pr.clone());
    }

    if problems.is_empty() {
        p.pass(t!(
            l,
            "{} events, well-formed{}",
            "{} 个事件，格式规范{}",
            names.len(),
            if s.saw_done_sentinel {
                t!(l, ", [DONE] present", "，[DONE] 正常")
            } else {
                String::new()
            }
        ))
        .took(took)
    } else if s.text.trim().is_empty() {
        p.fail(t!(
            l,
            "Streaming unusable: {}",
            "流式响应不可用：{}",
            problems.join("；")
        ))
        .took(took)
    } else {
        p.warn(t!(
            l,
            "Content arrived, but the format is wrong: {}",
            "内容拿到了，但格式有问题：{}",
            problems.join("；")
        ))
        .took(took)
    }
}

pub async fn stream_not_empty(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "stream_body",
        ts!(l, "Stream carries a body", "流式非空 body"),
        G,
    )
    .weight(3);
    let t0 = now_ms();

    let s = match ctx.client.stream(&stream_req(ctx)).await {
        Ok(s) => s,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    let took = (now_ms() - t0) as u64;

    let p = p
        .metric("bytes", s.bytes)
        .metric("text_len", s.text.chars().count())
        .metric("status", s.status)
        .evidence(crate::util::truncate(&s.text, 200));

    if s.bytes == 0 {
        return p
            .fail(t!(l, "The stream opened and delivered zero bytes", "流打开了但一个字节都没有"))
            .finding(t!(l, "The most common relay failure: the connection opens, keep-alive holds it, content never arrives", "这是中转站最高发的故障：连接建立、keep-alive 维持、内容永远不来"))
            .took(took);
    }
    if s.text.trim().is_empty() {
        return p
            .fail(t!(l, "Received {} bytes of events but no text content", "收到 {} 字节事件，但没有任何文本内容", s.bytes))
            .finding(t!(l, "The event frames exist but every delta is empty — upstream may have refused and a middle layer swallowed the error", "事件框架存在但 delta 全空——上游可能拒答后被中间层吞掉了错误"))
            .took(took);
    }
    p.pass(t!(
        l,
        "{} bytes / {} characters of content",
        "{} 字节 / {} 字符内容",
        s.bytes,
        s.text.chars().count()
    ))
    .took(took)
}

pub async fn stream_usage(ctx: &Ctx) -> ProbeResult {
    let l = ctx.lang;
    let p = ProbeResult::new(
        "stream_usage",
        ts!(l, "Streamed usage reporting", "流式 usage 上报"),
        G,
    )
    .weight(1);
    let t0 = now_ms();

    let s = match ctx.client.stream(&stream_req(ctx)).await {
        Ok(s) => s,
        Err(e) => return p.error(format!("{e}")).took((now_ms() - t0) as u64),
    };
    let took = (now_ms() - t0) as u64;

    match &s.usage {
        Some(u) if u.output_tokens > 0 => {
            let est = crate::util::estimate_tokens(&s.text);
            let p = p
                .metric("usage_absent", false)
                .metric("input_tokens", u.input_tokens)
                .metric("output_tokens", u.output_tokens)
                .metric("estimated_output_tokens", est);
            // Compared against a local heuristic, so this needs both a wide
            // ratio and enough absolute tokens: a 12-token answer of digits
            // and commas estimates badly no matter what the endpoint does.
            if est > 0 && u.output_tokens > 60 && u.output_tokens as f64 > est as f64 * 3.0 {
                p.warn(t!(
                    l,
                    "Stream reported {} output tokens; local estimate is only {est}",
                    "流式上报 {} 输出 token，本地估算仅 {est}",
                    u.output_tokens
                ))
                .finding(t!(
                    l,
                    "Far outside tokenizer variance; read this together with the billing audit",
                    "差距远超分词器误差范围，需结合计量审计一起看"
                ))
                .took(took)
            } else {
                p.pass(t!(
                    l,
                    "usage reported normally: {} in / {} out",
                    "usage 正常上报：输入 {} / 输出 {}",
                    u.input_tokens,
                    u.output_tokens
                ))
                .took(took)
            }
        }
        Some(_) => p
            .metric("usage_absent", false)
            .warn(t!(
                l,
                "usage was reported but output tokens are zero",
                "上报了 usage 但输出 token 为 0"
            ))
            .took(took),
        None => p
            .metric("usage_absent", true)
            .warn(t!(
                l,
                "The stream carried no usage at all",
                "流式响应完全没有 usage"
            ))
            .finding(t!(
                l,
                "Billing cannot be checked in streaming mode; common in reconstructed channels",
                "无法在流式模式下核对计费；逆向渠道常见特征"
            ))
            .took(took),
    }
}
