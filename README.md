# llm-verify

检测你正在用的 LLM 端点：模型真假、计费掺水、中转来源、性能与降智。

单二进制，无运行时依赖。检测结果输出为一份可以直接用浏览器打开的 HTML 报告。

## 安装

```bash
curl -fsSL https://raw.githubusercontent.com/asale-ai/llm-verify/main/install.sh | sh
```

Windows 用 `irm https://raw.githubusercontent.com/asale-ai/llm-verify/main/install.ps1 | iex`。

## 用法

```bash
llm-verify --base-url https://api.anthropic.com \
           --api-key sk-ant-... \
           --model claude-opus-4-5
```

跑完会生成一份 HTML 报告并自动在浏览器打开。

凭据也可以放进 `.env` 或环境变量，这样命令行就只剩模型名：

```bash
# .env
LLM_VERIFY_BASE_URL=https://api.anthropic.com
LLM_VERIFY_API_KEY=sk-ant-...
LLM_VERIFY_MODEL=claude-opus-4-5
```

```bash
llm-verify
```

### 常用参数

| 参数 | 说明 |
|---|---|
| `--protocol anthropic\|openai` | 不填会根据 URL 与模型名自动推断 |
| `--depth fast\|balanced\|forensic` | 默认 `balanced`。`forensic` 采样更多、更准也更贵 |
| `--claimed-model <ID>` | 供应商宣称的模型名与实际请求的不同时用，用于查降级 |
| `-o <path>` | HTML 报告路径，传目录则自动命名 |
| `--json <path>` | 同时输出机器可读的 JSON |
| `--no-open` | 不自动打开浏览器 |

### 退出码

| 码 | 含义 |
|---|---|
| 0 | 干净 |
| 1 | 评分不及格，或判定为存疑 / 假冒 / 无法判定 |
| 2 | 命中硬门禁 |

可以直接当 CI 门禁用。

## 报告怎么读

结论分两条**互相独立**的轴：

- **真伪**：正品 / 正品（有瑕疵）/ 第三方转发 / 存疑 / 假冒 / 无法判定
- **来源**：官方直连 / 云平台 / 订阅号 / 普通中转 / 逆向渠道 / 无法确定

一个经过中转的真模型，真伪是「第三方转发」而不是「假冒」——链路更长，但模型没被换。

**硬门禁**是加权分掩盖不掉的硬事实，命中任意一条直接判定存疑并以退出码 2 结束：

静默 fallback · 共享池裸转发 · 档位降级 · 第三方壳注入 · 缓存回放 · 隐藏 prompt 注入 · 响应重放

## 在 AI 编程工具里用

技能发布在 [ClawHub](https://clawhub.ai)：

```bash
clawhub install @asale-ai/llm-verify
```

或者用自带的安装器，一次写入所有已检测到的工具（Claude Code / Codex CLI / OpenCode / Gemini CLI）：

```bash
llm-verify install-skill
```

装好之后直接问「帮我看看这个 API 是不是真的」「这个中转站有没有多收钱」就会自动调用。

```bash
llm-verify skill-targets              # 看会装到哪些位置
llm-verify install-skill -t claude    # 只装一个
llm-verify install-skill --project    # 装到当前项目
```

技能本身只是使用说明，实际检测由 `llm-verify` 二进制执行，所以两者都需要。

## 能测什么

40 项探针，分七组：

| 组 | 回答的问题 |
|---|---|
| 协议契约 | 这是不是一条正牌 API 通道 |
| 流式传输 | 流式响应是否规范，有没有空 body |
| 计量计费 | 计费数字可信吗，有没有多收钱 |
| 渠道溯源 | 这条链路上有哪些中转 |
| 性能速度 | 首字延迟、吞吐与抖动 |
| 模型身份 | 背后跑的是不是它声称的那个模型 |
| 跨请求一致性 | 多次请求的行为是否一致 |

## 能力边界

对诚实供应商的一次误判，代价远高于一次漏检。工具在证据不足时一律弃权，不猜结论。以下几条请一并了解：

- **只做到「档位」粒度**（旗舰 / 中档 / 轻量）。同档位内的相邻版本（如同系列的 4.5 与 4.6）无法区分。
- **能力高于宣称不算欺诈**，只有实测低于宣称才计入风险。
- **中间层注入会污染身份指纹**，所以协议契约层先跑，发现注入时身份结论会自动降权。
- **无法证明服务端权重就是官方权重**，只能证明行为与预期一致或不一致。
- **一次检测只代表此刻**，渐进式降级需要定期重跑对比。

## 许可

[Apache-2.0](LICENSE)
