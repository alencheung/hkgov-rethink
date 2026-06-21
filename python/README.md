# hkgov-py

A typed Python client for [hkgov-rethink](https://github.com/alencheung/hkgov-rethink) —
the AI-infused HKGOV data insights platform. Talk to the HTTP API without
writing a line of Rust.

## Install

```bash
pip install hkgov-py
```

## Usage

```python
from hkgov import HkGov

client = HkGov("http://localhost:8080")           # api_key="..." when auth is on

# What's ingested?
for s in client.sources():
    print(s.source, s.dataset, s.record_count, "—", s.title)

# Read records
page = client.records("hkma", "daily-interbank-liquidity", limit=5)
for r in page.records:
    print(r.record_id, r.fields.get("hibor_overnight"))

# AI-agent insights (the whole point)
for i in client.insights(limit=5):
    print(f"[{i.severity}] {i.title}\n    {i.summary}")
    for e in i.evidence:
        print(f"    evidence: {e.record_id} .{e.field} = {e.value}")

# Ask a question in natural language
answer = client.ask("what is the interbank liquidity doing?")
print(answer.text, f"({answer.confidence:.0%} confidence)")
for step in answer.trace:
    print(f"  tool: {step.tool}")
```

## Sync vs async

`hkgov.HkGov` is synchronous (uses `requests`). An async client is on the roadmap
(see the repo's good-first-issues). Until then, wrap calls in a thread or use
`asyncio.to_thread`.

## API surface

The client mirrors the HTTP API 1:1:

| Method | HTTP |
|---|---|
| `client.health()` | `GET /health` |
| `client.source_health()` | `GET /v1/health/sources` |
| `client.sources()` | `GET /v1/sources` |
| `client.dataset(source, dataset)` | `GET /v1/datasets/{source}/{dataset}` |
| `client.records(source, dataset, offset=0, limit=100)` | `GET /v1/datasets/{source}/{dataset}/records` |
| `client.insights(limit=20)` | `GET /v1/insights` |
| `client.alerts(limit=20)` | `GET /v1/alerts` |
| `client.ask(question)` | `POST /v1/ask` |

## Development

```bash
cd python
pip install -e ".[dev]"
pytest
ruff check .
mypy src/hkgov
```

## License

MIT, same as the parent project.
