# 贡献指南

[English](CONTRIBUTING.md) · **简体中文**

## 构建

需要 Rust 1.82 或更新版本。

```bash
cargo build            # debug
cargo build --release  # release，strip 后约 2 MB
cargo test             # 单元测试，不需要网络
cargo clippy -- -D warnings
cargo fmt --check
```

release 配置以体积优先（`opt-level = "z"`、fat LTO、`panic = "abort"`、strip）。这是个 I/O 密集的工具，体积比代码生成质量更重要。

## 依赖策略

产物要发到五个平台，而且用户是通过一行 `curl | sh` 装的，所以每加一个依赖都要先掂量体积。当前直接依赖：`clap`、`reqwest`、`tokio`、`futures-util`、`serde`、`serde_json`、`anyhow`。

以下是刻意没用的，理由写在 `src/util.rs` 里：

- **chrono / time** —— `iso8601_utc()` 就是 20 行 civil-from-days 算法。
- **rand** —— `Rng` 是 15 行 xorshift64\*。探针载荷只需要供应商猜不到，不需要密码学强度。
- **tiktoken** —— 它的 rank 表要好几 MB。端点提供 `count_tokens` 时我们用权威计数；否则用一个够用的启发式，并且由此得出的每个结论都会标注为「估算」。
- **彩色库** —— `src/term.rs` 直接写用到的那 8 个 ANSI 码。
- **模板引擎** —— `src/html.rs` 用 `write!` 拼一个 `const CSS`。

`reqwest` 用 `rustls-tls` 而非 OpenSSL，交叉编译时不需要任何系统库。

## 目录结构

```
src/
  main.rs        CLI、env/.env 解析、产物写出
  i18n.rs        Lang 枚举、locale 检测、t! / ts! 宏
  protocol.rs    OpenAI 与 Anthropic 两套线上格式
  client.rs      HTTP 传输与 SSE 解析
  probes/
    mod.rs       注册表、执行顺序、共享 Ctx
    contract.rs  协议契约（12）
    stream.rs    流式行为（3）
    billing.rs   计量与计费审计（7）
    identity.rs  模型身份与能力档位（8）
    consistency.rs 跨请求一致性（3）
    perf.rs      TTFT / 延迟 / 吞吐 / 抖动（4）
    channel.rs   渠道溯源（3）
  verdict.rs     评分、硬门禁、双轴判定
  report.rs      所有探针写入、所有输出读取的数据模型
  html.rs        自包含 HTML 报告
  term.rs        终端输出
  pricing.rs     内置定价表
  util.rs        时间、PRNG、token 估算、格式化
skills/
  llm-verify/SKILL.md   技能正文，用 `npx skills add` 安装
```

技能是仓库里的一个普通文件，不由二进制生成。要改就直接改
`skills/llm-verify/SKILL.md`，`npx skills add asale-ai/llm-verify` 从仓库读取它。

探针顺序执行、通过 `RefCell` 共享一个 `Ctx`。契约探针刻意排在最前：如果链路会重写请求，后面的指纹结论就不可靠，裁决层必须在读它们之前知道这件事。

## 双语输出

用户可见的每一句话都必须同时存在于两种语言里。文案就写在使用它的地方，两种语言并排：

```rust
p.pass(t!(l, "Endpoint reachable, {}ms", "端点可达，{}ms", raw.duration_ms))
```

- `t!` 返回 `String`，`ts!` 返回 `&'static str`。两半接收同一组参数，占位符对不上是编译错误。
- `t!` 即使没有额外参数也走 `format!`，因为文案大量使用 `{host}` 这类内联捕获。所以文案里的字面花括号必须写成 `{{`。
- 分类结果用**稳定的 key**，不要用显示名。裁决层按 key 路由，翻译永远不能改变判定结果——见 `probes/channel.rs`。
- `i18n::coverage_tests` 会在源码层面检查：任何中文字面量都必须有英文搭档。它抓到过一次真实事故——`contract.rs` 曾整份只有中文，英文模式下三分之一的探针默默输出中文。

## 加一个探针

1. 在对应的 `src/probes/*.rs` 里写一个返回 `ProbeResult` 的函数。
2. 在 `src/probes/mod.rs` 的 `run_all` 里注册。
3. 更新 `PROBE_COUNT`。
4. 如果裁决层要对它作出反应，在 `src/verdict.rs` 里按 ID 读取。

### 探针必须遵守的规则

这些规则存在，是因为本项目最坏的失败模式是冤枉一个诚实的供应商，而不是漏检。

- **端点单纯不支持某个能力时用 `Skip`，不要用 `Fail`。** 一个没有 `/models` 的中转不等于欺诈。
- **绝不在小数字上断言倍率。** 每请求的固定开销会让小 prompt 的倍率虚高：对着真实网关实测，6 token 的 prompt 显示 1.75×，而**同一个端点**上 458 token 的 prompt 只有 1.01×。比例之外还要卡绝对差额。
- **绝不用「模型拒绝了对抗性指令」来推断中间层故障。** 早期的 `system_adherence` 探针要求模型无视用户问题，较强的模型正当地拒绝了，探针却把这读成 system prompt 被丢弃。正确做法是在 system prompt 里放一个唯一标记再要回来。
- **方向很重要。** 实测能力**高于**宣称是超额交付，不是欺诈，绝不能触发门禁。
- **载荷每次运行现场生成。** 固定题目会被供应商预先缓存，用 `ctx.rng`。
- **只取证不计分的探针标 `.neutral()`**，这样它能给裁决层提供信息而不影响分数。

## 测试

单元测试是内联的 `#[cfg(test)]` 模块，不允许触网。最值得看的几个是护栏测试：`verdict::tests::family_mismatch_on_a_weak_signal_degrades_to_ambiguous`、`narrow_tier_margin_withdraws_the_severity_claim`、`reverse_weight_tests::weak_signals_alone_stay_below_the_threshold`、`i18n::coverage_tests::every_chinese_literal_has_an_english_partner`。

要对着真实端点做端到端验证，把凭据放进 `.env`（已被 git 忽略），并且先跑一个已知良好的供应商，这样工具的改动才和端点的变化区分得开。

## 发布

`publish.sh` 全程无人值守：

```bash
./publish.sh "commit message"          # patch
./publish.sh -m minor "commit message" # minor
./publish.sh --dry-run "message"       # 预演
```

它会升 `Cargo.toml` 的版本、提交、推送、打 tag、推 tag。tag 触发 `.github/workflows/release.yml`，交叉编译五个目标、连同 `SHA256SUMS` 打包，并创建 GitHub Release。

凭据从 `.env` 读，绝不写进仓库。
