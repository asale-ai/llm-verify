// SPDX-License-Identifier: Apache-2.0
//! Installs the llm-verify skill into whichever AI coding tools are present.
//!
//! Three of the four targets converged on the same shape — a directory holding
//! a `SKILL.md` with YAML frontmatter — so one document serves them all. Gemini
//! CLI has no skill concept and only takes TOML slash commands, so its content
//! is generated from the same source rather than maintained separately.

use crate::util::pad_display;
use anyhow::{Context, Result};
use std::path::PathBuf;

const SKILL_NAME: &str = "llm-verify";
const DESCRIPTION: &str = "检测 LLM API 端点的真伪、计费掺水、中转来源、性能与降智。当用户询问\"我用的模型是不是真的\"、\"这个中转站靠谱吗\"、\"是不是被降智了\"、\"计费对不对\"，或要求检测/验证某个 API 端点时使用。";

#[derive(clap::Args, Clone)]
pub struct InstallArgs {
    /// 安装目标，可重复或逗号分隔：claude / codex / opencode / gemini / agents / all
    #[arg(long, short = 't', default_value = "all", value_delimiter = ',')]
    target: Vec<String>,

    /// 安装到当前项目而非用户主目录
    #[arg(long)]
    project: bool,

    /// 只打印将要写入的路径，不实际写入
    #[arg(long)]
    dry_run: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// A directory containing SKILL.md with YAML frontmatter.
    SkillMd,
    /// A single TOML file describing one slash command.
    GeminiToml,
}

#[derive(Debug, Clone)]
pub struct Target {
    pub key: &'static str,
    pub tool: &'static str,
    pub format: Format,
    /// Path relative to `$HOME`.
    pub user_dir: &'static str,
    /// Path relative to the project root.
    pub project_dir: &'static str,
}

pub const TARGETS: &[Target] = &[
    Target {
        key: "claude",
        tool: "Claude Code",
        format: Format::SkillMd,
        user_dir: ".claude/skills",
        project_dir: ".claude/skills",
    },
    Target {
        key: "codex",
        tool: "Codex CLI",
        format: Format::SkillMd,
        user_dir: ".codex/skills",
        project_dir: ".codex/skills",
    },
    Target {
        key: "opencode",
        tool: "OpenCode",
        format: Format::SkillMd,
        user_dir: ".config/opencode/skills",
        project_dir: ".opencode/skills",
    },
    Target {
        key: "agents",
        tool: "通用 agents 目录",
        format: Format::SkillMd,
        user_dir: ".agents/skills",
        project_dir: ".agents/skills",
    },
    Target {
        key: "gemini",
        tool: "Gemini CLI",
        format: Format::GeminiToml,
        user_dir: ".gemini/commands",
        project_dir: ".gemini/commands",
    },
];

/// Expand the requested target list, resolving `all`.
pub fn resolve_targets(requested: &[String]) -> Vec<&'static Target> {
    let wants_all = requested
        .iter()
        .any(|t| t.trim().eq_ignore_ascii_case("all"));
    if wants_all || requested.is_empty() {
        return TARGETS.iter().collect();
    }
    let mut out = Vec::new();
    for want in requested {
        let want = want.trim().to_ascii_lowercase();
        if let Some(t) = TARGETS.iter().find(|t| t.key == want) {
            if !out.iter().any(|e: &&Target| e.key == t.key) {
                out.push(t);
            }
        }
    }
    out
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// Where a target's payload lands, and what it is called.
pub fn destination(t: &Target, project: bool) -> Option<PathBuf> {
    let base = if project {
        std::env::current_dir().ok()?.join(t.project_dir)
    } else {
        home()?.join(t.user_dir)
    };
    Some(match t.format {
        Format::SkillMd => base.join(SKILL_NAME).join("SKILL.md"),
        Format::GeminiToml => base.join(format!("{SKILL_NAME}.toml")),
    })
}

pub fn install(args: &InstallArgs) -> Result<()> {
    let targets = resolve_targets(&args.target);
    if targets.is_empty() {
        anyhow::bail!(
            "没有匹配的安装目标。可选：{}",
            TARGETS
                .iter()
                .map(|t| t.key)
                .collect::<Vec<_>>()
                .join(" / ")
        );
    }

    println!();
    println!(
        "安装 llm-verify 技能{}",
        if args.dry_run { "（预演）" } else { "" }
    );
    println!();

    let mut written = 0usize;
    let mut skipped = Vec::new();

    for t in targets {
        let Some(dest) = destination(t, args.project) else {
            skipped.push(format!("{}：无法确定主目录", t.tool));
            continue;
        };
        let content = match t.format {
            Format::SkillMd => skill_md(),
            Format::GeminiToml => gemini_toml(),
        };

        if args.dry_run {
            println!("  [预演] {} → {}", pad_display(t.tool, 20), dest.display());
            written += 1;
            continue;
        }

        // Only create the tool's own tree — never invent a whole config root
        // for a tool the user does not have installed.
        let tool_root = tool_root(t, args.project);
        let tool_root_exists = tool_root.as_ref().map(|p| p.exists()).unwrap_or(false);
        if !tool_root_exists && !args.project {
            skipped.push(format!(
                "{}：未检测到 {}，已跳过",
                t.tool,
                tool_root
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
            continue;
        }

        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("无法创建 {}", parent.display()))?;
        }
        std::fs::write(&dest, content).with_context(|| format!("无法写入 {}", dest.display()))?;
        println!("  ✓ {} → {}", pad_display(t.tool, 20), dest.display());
        written += 1;
    }

    for s in &skipped {
        println!("  – {s}");
    }

    println!();
    if written == 0 {
        println!(
            "没有写入任何文件。用 --target claude 之类指定目标，或用 --project 装到当前项目。"
        );
    } else {
        println!("已写入 {written} 个位置。重启对应工具后即可使用。");
        println!("Gemini CLI 通过 /llm-verify 调用，其余工具会在相关问题上自动加载该技能。");
    }
    Ok(())
}

/// The directory whose existence proves the tool is installed.
fn tool_root(t: &Target, project: bool) -> Option<PathBuf> {
    let first = t
        .user_dir
        .split('/')
        .take(if t.user_dir.starts_with(".config") {
            2
        } else {
            1
        })
        .collect::<Vec<_>>()
        .join("/");
    if project {
        std::env::current_dir().ok()
    } else {
        home().map(|h| h.join(first))
    }
}

pub fn list_targets() -> Result<()> {
    println!();
    println!("llm-verify 技能安装位置");
    println!();
    println!(
        "  {} {} {} 用户级路径",
        pad_display("KEY", 10),
        pad_display("工具", 20),
        pad_display("格式", 12)
    );
    println!("  {}", "─".repeat(76));
    for t in TARGETS {
        let installed = tool_root(t, false).map(|p| p.exists()).unwrap_or(false);
        println!(
            "  {} {} {} {}{}",
            pad_display(t.key, 10),
            pad_display(t.tool, 20),
            pad_display(
                match t.format {
                    Format::SkillMd => "SKILL.md",
                    Format::GeminiToml => "TOML 命令",
                },
                12
            ),
            pad_display(&format!("~/{}", t.user_dir), 30),
            if installed { "✓ 已检测到" } else { "" }
        );
    }
    println!();
    println!("  安装：llm-verify install-skill              （所有已检测到的工具）");
    println!("       llm-verify install-skill -t claude,codex");
    println!("       llm-verify install-skill --project     （装到当前项目）");
    println!();
    Ok(())
}

// ── payloads ───────────────────────────────────────────────────────────────

/// Shared skill body. Written once, used by all three SKILL.md targets.
fn instructions() -> String {
    r#"
`llm-verify` 是一个单二进制 CLI，对任意 LLM API 端点做黑盒检测，回答用户真正关心的问题：
我用的到底是不是它声称的那个模型？有没有被多收钱？这条链路上有几层中转？

## 什么时候用

用户出现下面这类诉求时，主动使用本技能：

- 「我这个中转站靠谱吗」「这个 key 是不是真的 Claude」
- 「模型是不是被降智了」「怎么感觉变笨了」
- 「计费对不对」「token 数好像不对劲」
- 「帮我测一下这个 API 端点」
- 排查某个端点响应慢、流式断流、工具调用不工作

## 怎么用

```bash
llm-verify --base-url <URL> --api-key <KEY> --model <MODEL_ID>
```

常用参数：

| 参数 | 说明 |
|---|---|
| `--protocol anthropic\|openai` | 不填会自动推断 |
| `--depth fast\|balanced\|forensic` | 默认 balanced；forensic 采样更多、更贵也更准 |
| `--claimed-model <ID>` | 供应商宣称的模型名与实际请求的不同时使用 |
| `-o report.html` | 指定 HTML 报告路径 |
| `--json report.json` | 同时输出机器可读的 JSON |
| `--no-open` | 不自动打开浏览器 |

凭据也可以放进 `.env` 或环境变量：`LLM_VERIFY_BASE_URL`、`LLM_VERIFY_API_KEY`、`LLM_VERIFY_MODEL`。

## 退出码

| 码 | 含义 |
|---|---|
| 0 | 干净 |
| 1 | 评分不及格，或判定为存疑/假冒 |
| 2 | 命中硬门禁（静默 fallback、共享池裸转发、档位降级、壳注入、缓存回放、隐藏 prompt、响应重放） |

可直接用作 CI 门禁。

## 怎么读结果

工具输出两条**互相独立**的轴，不要混为一谈：

- **真伪**：正品 / 正品（有瑕疵）/ 第三方转发 / 存疑 / 假冒 / 无法判定
- **来源**：官方直连 / 云平台 / 订阅号 / 普通中转 / 逆向渠道 / 无法确定

一个经过中转的真模型，真伪是「第三方转发」而不是「假冒」——这不是问题，只是链路更长。

## 向用户汇报时的原则

1. **先说结论，再说证据。** 用户要的是「能不能用」，不是 38 项探针明细。
2. **硬门禁必须单独点出来。** 它们是加权分掩盖不掉的硬事实。
3. **未测 ≠ 通过。** 报告里的「跳过」项要如实说明，不要让用户以为全测过了。
4. **置信度要带上。** 尤其是身份判定——同家族相邻版本本来就难分辨，工具在证据不足时会主动弃权，这时不要替它下结论。
5. **不要替供应商定罪。** 「档位相差一档」在采样噪声范围内，工具不会指控，你也不要。

## 能力边界（必须如实告知）

- 同家族相邻版本（如 Opus 4.5 vs 4.6）在没有分布基线时不可分，只能给「家族相符、档位未验证」。
- 中间层注入的 system prompt 会污染风格指纹，所以协议契约层必须先跑；有注入时后面的身份结论要降权。
- 无法证明「服务端权重就是官方权重」，只能证明「行为与预期一致或不一致」。
- 量化版本（int4/fp8）与原版的区分只能给概率，给不了定论。
"#
    .trim()
    .to_string()
}

fn skill_md() -> String {
    format!(
        "---\nname: {SKILL_NAME}\ndescription: {DESCRIPTION}\nlicense: Apache-2.0\n---\n\n# llm-verify\n\n{}\n",
        instructions()
    )
}

fn gemini_toml() -> String {
    // TOML *literal* strings, not basic strings.
    //
    // The skill body is markdown whose tables contain `\|` inside cells. A
    // basic string reads that backslash as the start of an escape sequence and
    // the whole file fails to parse — which is exactly what an earlier version
    // shipped. Literal strings perform no escape processing at all, so the
    // body survives byte for byte. The only sequence a literal cannot hold is
    // its own delimiter, which the helpers below take care of.
    format!(
        "description = {}\n\nprompt = '''\n{}\n'''\n",
        toml_literal(DESCRIPTION),
        escape_for_toml_literal(&instructions())
    )
}

/// Wrap a single-line value as a TOML literal string. Falls back to a basic
/// string when the value contains an apostrophe, which a literal cannot hold.
/// The descriptions here use full-width quotation marks, so the fallback is
/// defensive rather than expected.
fn toml_literal(s: &str) -> String {
    if s.contains('\'') {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        format!("'{s}'")
    }
}

/// A multi-line literal string ends at the first `'''`. The body does not
/// contain one today; this keeps a future edit from silently producing an
/// unparseable file.
fn escape_for_toml_literal(body: &str) -> String {
    body.replace("'''", "''\u{200B}'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_expands_to_every_target() {
        assert_eq!(resolve_targets(&["all".into()]).len(), TARGETS.len());
        assert_eq!(resolve_targets(&[]).len(), TARGETS.len());
    }

    #[test]
    fn explicit_targets_are_deduplicated_and_filtered() {
        let got = resolve_targets(&[
            "claude".into(),
            "CLAUDE".into(),
            " codex ".into(),
            "not-a-tool".into(),
        ]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].key, "claude");
        assert_eq!(got[1].key, "codex");
    }

    #[test]
    fn target_keys_are_unique() {
        let mut keys: Vec<&str> = TARGETS.iter().map(|t| t.key).collect();
        let n = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), n);
    }

    #[test]
    fn skill_md_targets_use_a_directory_named_after_the_skill() {
        // OpenCode requires the frontmatter name to match the directory name;
        // the other two tolerate it, so one layout serves all three.
        for t in TARGETS.iter().filter(|t| t.format == Format::SkillMd) {
            let d = destination(t, false).unwrap();
            assert_eq!(d.file_name().unwrap(), "SKILL.md", "{}", t.key);
            assert_eq!(
                d.parent().unwrap().file_name().unwrap(),
                SKILL_NAME,
                "{}",
                t.key
            );
        }
    }

    #[test]
    fn gemini_target_writes_a_single_toml_file() {
        let t = TARGETS.iter().find(|t| t.key == "gemini").unwrap();
        let d = destination(t, false).unwrap();
        assert_eq!(d.file_name().unwrap(), "llm-verify.toml");
    }

    #[test]
    fn skill_md_frontmatter_is_well_formed() {
        let md = skill_md();
        assert!(md.starts_with("---\n"));
        let end = md[4..].find("\n---\n").expect("frontmatter must terminate");
        let fm = &md[4..4 + end];
        assert!(fm.contains(&format!("name: {SKILL_NAME}")));
        assert!(fm.contains("description: "));
        // The name must satisfy OpenCode's constraint: lowercase alphanumeric
        // with single hyphens, no leading/trailing hyphen.
        assert!(SKILL_NAME
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'));
        assert!(!SKILL_NAME.starts_with('-') && !SKILL_NAME.ends_with('-'));
        assert!(!SKILL_NAME.contains("--"));
        assert!(SKILL_NAME.len() <= 64);
    }

    #[test]
    fn gemini_toml_parses_and_round_trips_the_body() {
        let toml = gemini_toml();
        assert!(toml.starts_with("description = '"));
        assert!(toml.contains("prompt = '''"));
        assert_eq!(
            toml.matches("'''").count(),
            2,
            "a literal string must have exactly its opening and closing delimiter"
        );

        let (desc_line, rest) = toml.split_once('\n').unwrap();
        let desc = desc_line
            .strip_prefix("description = '")
            .and_then(|s| s.strip_suffix('\''))
            .expect("description must be a closed literal string");
        assert!(!desc.is_empty());
        assert!(
            !desc.contains('\''),
            "an apostrophe would close the literal early"
        );

        let body = rest
            .split_once("prompt = '''\n")
            .and_then(|(_, b)| b.strip_suffix("'''\n"))
            .expect("prompt must be a closed multi-line literal")
            .strip_suffix('\n')
            .unwrap();
        assert_eq!(body, instructions(), "body must survive verbatim");
    }

    #[test]
    fn the_body_still_contains_the_backslash_that_broke_basic_strings() {
        // Regression guard. `\|` inside a markdown table cell is what made the
        // generated TOML unparseable under basic strings. If the body ever
        // loses it, the round-trip test above stops proving anything.
        assert!(instructions().contains(r"\|"));
    }

    #[test]
    fn toml_literal_falls_back_when_an_apostrophe_is_present() {
        assert_eq!(toml_literal("plain"), "'plain'");
        // A literal string may hold backslashes and double quotes untouched;
        // only an apostrophe forces the basic-string fallback.
        assert_eq!(toml_literal(r#"a\b"c"#), "'a\\b\"c'");
        assert_eq!(toml_literal("it's"), "\"it's\"");
        // Once in the fallback, backslashes and quotes must be escaped.
        assert_eq!(toml_literal(r#"it's a\b"c"#), r#""it's a\\b\"c""#);
    }

    #[test]
    fn a_triple_quote_in_the_body_cannot_terminate_the_string_early() {
        let escaped = escape_for_toml_literal("before ''' after");
        assert!(!escaped.contains("'''"));
        assert!(escaped.contains("before") && escaped.contains("after"));
    }

    #[test]
    fn both_payloads_carry_the_capability_limits() {
        // The limits section is the part that stops an agent overselling the
        // result; it must survive both format conversions.
        for payload in [skill_md(), gemini_toml()] {
            assert!(payload.contains("能力边界"));
            assert!(payload.contains("未测 ≠ 通过"));
            assert!(payload.contains("退出码"));
        }
    }
}
