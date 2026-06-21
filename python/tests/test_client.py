"""Tests for the hkgov-py client.

Uses `responses` to mock the requests transport so no real server is needed.
Run with:
    pip install -e ".[dev]"
    pytest
"""

from __future__ import annotations

import responses

from hkgov import HkGov, HkGovError

BASE = "http://localhost:8080"
PREFIX = "/v1"

SAMPLE_HEALTH = {"status": "ok", "version": "0.1.0"}
SAMPLE_SOURCE_HEALTH = [
    {"source": "hkma", "circuit": "closed"},
    {"source": "press", "circuit": "open"},
]
SAMPLE_SOURCES = [
    {
        "source": "hkma",
        "dataset": "daily-interbank-liquidity",
        "title": "Daily Interbank Liquidity",
        "description": "Daily figures.",
        "category": "monetary",
        "tags": ["hibor", "liquidity"],
        "cadence": "daily",
        "refresh_interval_secs": 3600,
        "last_refreshed_at": "2026-06-21T00:00:00Z",
        "record_count": 90,
    }
]
SAMPLE_CATEGORIES = [
    {"category": "monetary", "count": 2, "datasets": ["hkma/a", "hkma/b"]},
    {"category": "fiscal", "count": 1, "datasets": ["datagovhk/c"]},
]
SAMPLE_RECORDS = {
    "source": "hkma",
    "dataset": "daily-interbank-liquidity",
    "total": 90,
    "offset": 0,
    "limit": 2,
    "records": [
        {"record_id": "2026-06-18", "fields": {"hibor_overnight": 2.4}},
        {"record_id": "2026-06-17", "fields": {"hibor_overnight": 2.3}},
    ],
}
SAMPLE_INSIGHTS = [
    {
        "id": "series_jump:hkma:daily-interbank-liquidity:abc",
        "kind": "series_jump",
        "severity": "critical",
        "title": "hibor_overnight moved +99.3%",
        "summary": "HIBOR doubled in one session.",
        "source": "hkma",
        "dataset": "daily-interbank-liquidity",
        "evidence": [
            {"record_id": "2026-02-13", "field": "hibor_overnight", "value": 1.47},
            {"record_id": "2026-02-16", "field": "hibor_overnight", "value": 2.93},
        ],
        "confidence": 0.9,
        "generated_at": "2026-06-21T00:00:00Z",
        "producer": "heuristic",
    }
]
SAMPLE_ANSWER = {
    "text": "HIBOR doubled on 2026-02-16.",
    "confidence": 0.8,
    "trace": [{"tool": "run_detector", "arguments": {"detector": "series_jump"}, "result": {}}],
}
SAMPLE_ALERTS = [
    {
        "insight_id": "series_jump:hkma:x:1",
        "insight_kind": "series_jump",
        "severity": "critical",
        "sink": "webhook",
        "status": "ok",
        "dispatched_at": "2026-06-21T00:00:00Z",
    }
]


def _client(**kw) -> HkGov:
    return HkGov(BASE, **kw)


@responses.activate
def test_health() -> None:
    responses.add(responses.GET, f"{BASE}/health", json=SAMPLE_HEALTH, status=200)
    h = _client().health()
    assert h.status == "ok"
    assert h.version == "0.1.0"


@responses.activate
def test_sources() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/sources", json=SAMPLE_SOURCES, status=200)
    s = _client().sources()
    assert len(s) == 1
    assert s[0].source == "hkma"
    assert s[0].record_count == 90
    assert s[0].category == "monetary"
    assert "hibor" in s[0].tags
    assert s[0].cadence == "daily"


@responses.activate
def test_sources_filters_pass_query_params() -> None:
    # The filter kwargs must translate to the right query params.
    route = responses.add(
        responses.GET, f"{BASE}{PREFIX}/sources", json=SAMPLE_SOURCES, status=200
    )
    _client().sources(category="monetary", tag=["hibor", "liquidity"], cadence="daily", q="interbank")
    sent = responses.calls[-1].request
    assert "category=monetary" in sent.url
    assert "cadence=daily" in sent.url
    assert "q=interbank" in sent.url
    # repeated tag params
    assert "tag=hibor" in sent.url and "tag=liquidity" in sent.url


@responses.activate
def test_sources_single_tag_string() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/sources", json=SAMPLE_SOURCES, status=200)
    _client().sources(tag="hibor")
    sent = responses.calls[-1].request
    assert "tag=hibor" in sent.url


@responses.activate
def test_categories() -> None:
    responses.add(
        responses.GET, f"{BASE}{PREFIX}/categories", json=SAMPLE_CATEGORIES, status=200
    )
    cats = _client().categories()
    assert len(cats) == 2
    monetary = next(c for c in cats if c.category == "monetary")
    assert monetary.count == 2
    assert len(monetary.datasets) == 2


@responses.activate
def test_records_pagination() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/datasets/hkma/daily-interbank-liquidity/records",
        json=SAMPLE_RECORDS,
        status=200,
    )
    page = _client().records("hkma", "daily-interbank-liquidity", offset=0, limit=2)
    assert page.total == 90
    assert len(page.records) == 2
    assert page.records[0].fields["hibor_overnight"] == 2.4
    # Query params were sent.
    sent = responses.calls[-1].request
    assert "limit=2" in sent.url


@responses.activate
def test_insights_with_evidence() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/insights", json=SAMPLE_INSIGHTS, status=200)
    insights = _client().insights(limit=5)
    assert insights[0].severity == "critical"
    assert len(insights[0].evidence) == 2
    assert insights[0].evidence[0].value == 1.47


@responses.activate
def test_ask_returns_answer_with_trace() -> None:
    responses.add(responses.POST, f"{BASE}{PREFIX}/ask", json=SAMPLE_ANSWER, status=200)
    a = _client().ask("what happened to hibor?")
    assert "doubled" in a.text
    assert a.confidence == 0.8
    assert len(a.trace) == 1
    assert a.trace[0].tool == "run_detector"


@responses.activate
def test_alerts() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/alerts", json=SAMPLE_ALERTS, status=200)
    alerts = _client().alerts()
    assert alerts[0].severity == "critical"
    assert alerts[0].status == "ok"


@responses.activate
def test_source_health() -> None:
    responses.add(
        responses.GET, f"{BASE}{PREFIX}/health/sources", json=SAMPLE_SOURCE_HEALTH, status=200
    )
    sh = _client().source_health()
    assert sh[1].source == "press"
    assert sh[1].circuit == "open"


@responses.activate
def test_error_on_non_2xx() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/sources", json={"error": "down"}, status=503)
    import pytest

    with pytest.raises(HkGovError, match="503"):
        _client().sources()


@responses.activate
def test_api_key_header_sent() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/sources", json=[], status=200)
    _client(api_key="secret").sources()
    assert responses.calls[-1].request.headers.get("X-API-Key") == "secret"
