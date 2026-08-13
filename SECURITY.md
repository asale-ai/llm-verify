# Security Policy

**English** · [简体中文](SECURITY.zh-CN.md)

## Reporting a vulnerability

Report privately through [GitHub Security Advisories](https://github.com/asale-ai/llm-verify/security/advisories/new). Please do not open a public issue for a vulnerability.

Include what you can: affected version, reproduction steps, and impact. Expect an acknowledgement within 5 working days and an assessment within 15.

## Supported versions

The latest released minor version receives fixes. Older versions do not.

## How this tool handles your credentials

`llm-verify` sends your API key to exactly one place: the endpoint you point it at.

- The key is held in process memory only. It is never written to disk, never logged, and never included in the HTML or JSON report.
- `.env` is read directly and **not** exported into the process environment, so a key cannot leak into a child process.
- HTTP redirects are disabled. A relay cannot bounce a probe — and its `Authorization` header — to a host you did not choose.
- TLS uses `rustls` with the platform trust store. There is no option to skip certificate verification.

Reports are written to the local filesystem only. Nothing is uploaded anywhere.

## What the reports contain

Reports include response excerpts from the endpoint under test, response headers, message IDs, and token counts. Treat a report as you would the conversation itself.

Reports never contain your API key. They do contain the base URL and model ID.

## Probe safety

Probes are read-only with respect to the endpoint: they send chat requests and read responses. They do not attempt to escalate privileges, exfiltrate data, or bypass provider safety systems.

**Probe content is deliberately benign.** A related open-source project shipped refusal-gradient probes containing genuinely harmful categories; once upstream providers tightened content controls, running that suite got users' accounts suspended. Any probe added here must be constructed from harmless material. A probe that could plausibly trip a provider's abuse controls is a defect, not a feature — see the rules in [CONTRIBUTING.md](CONTRIBUTING.md).

Three probes intentionally send malformed requests — omitting the API version header, omitting authentication, and naming a model that cannot exist. These are single well-formed HTTP requests that a compliant endpoint rejects. They are not attacks and do not generate load.

## Scope

Only test endpoints you are authorised to test. `llm-verify` sends real billable requests: roughly 40–60 per run depending on `--depth`. Pointing it at infrastructure you do not own or have permission to assess may violate the provider's terms of service.

## Interpreting results

Reports are automated black-box measurements. They are evidence, not proof, and explicitly not a legal accusation against any provider. The tool abstains rather than guessing when signals are weak; see the capability limits in the report and in [README.md](README.md).
