"""Tests for the hkgov-py client.

Uses `responses` to mock the requests transport so no real server is needed.
Run with:
    pip install -e ".[dev]"
    pytest
"""

from __future__ import annotations

import responses
import pytest

from hkgov import Answer, HkGov, HkGovError, TraceStep

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
# A brief item flattens the insight fields onto the top level alongside
# rank/score (mirrors the Rust BriefItem #[serde(flatten)]).
SAMPLE_BRIEF = {
    "generated_at": "2026-06-21T00:00:00Z",
    "items": [
        {
            "rank": 1,
            "score": 100.0,
            **SAMPLE_INSIGHTS[0],
        }
    ],
}


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
    responses.add(
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
    with pytest.raises(HkGovError, match="503"):
        _client().sources()


@responses.activate
def test_api_key_header_sent() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/sources", json=[], status=200)
    _client(api_key="secret").sources()
    assert responses.calls[-1].request.headers.get("X-API-Key") == "secret"


@responses.activate
def test_brief_re_nests_flattened_insight() -> None:
    responses.add(responses.GET, f"{BASE}{PREFIX}/brief", json=SAMPLE_BRIEF, status=200)
    brief = _client().brief(limit=5)
    assert brief.generated_at == "2026-06-21T00:00:00Z"
    assert len(brief.items) == 1
    item = brief.items[0]
    assert item.rank == 1
    assert item.score == 100.0
    # The flattened insight fields are re-nested under .insight.
    assert item.insight.severity == "critical"
    assert item.insight.title.startswith("hibor_overnight")
    assert len(item.insight.evidence) == 2


@responses.activate
def test_feedback_posts_and_reads_score() -> None:
    insight_id = SAMPLE_INSIGHTS[0]["id"]
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/insights/{insight_id}/feedback",
        json={"recorded": True},
        status=200,
    )
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/insights/{insight_id}/feedback",
        json={"insight_id": insight_id, "net_useful": 1},
        status=200,
    )
    c = _client()
    c.feedback(insight_id, useful=True, note="great catch")
    # The POST body must carry useful + note.
    sent_body = responses.calls[0].request.body
    assert b'"useful": true' in sent_body
    assert b'"note": "great catch"' in sent_body
    assert c.feedback_score(insight_id) == 1


# ── v7/v8 endpoint coverage (D-011) ──────────────────────────────────────────

SAMPLE_SILENCE_INDEX = {
    "label": "HKMA Silence Index",
    "methodology_version": "1.0",
    "source": "hkma",
    "period": "2026-Q2",
    "score": 75.76,
    "raw_score": 120.0,
    "computed_at": "2026-07-01T00:00:00Z",
    "total_events": 25,
    "signals": [
        {
            "kind": "unattributed_series_jump",
            "count": 20,
            "weight": 5.0,
            "contribution": 100.0,
            "evidence_ids": ["series_jump:hkma:a", "series_jump:hkma:b"],
        }
    ],
}

SAMPLE_UNPRECEDENTEDNESS = {
    "value": 2.93,
    "n": 90,
    "percentile": 99.5,
    "band": {"low": 0.5, "high": 2.0, "median": 1.2, "mad": 0.3},
    "one_in_n": 200,
    "hist_min": 0.1,
    "hist_max": 3.1,
    "last_exceeded": {
        "record_id": "2024-12-15",
        "value": 3.1,
        "when": "2024-12-15T00:00:00Z",
        "pct_beyond_edge": 55.0,
    },
}

SAMPLE_CITATION = {
    "permalink": "/cite/series_jump:hkma:abc",
    "insight_id": "series_jump:hkma:abc",
    "cite_version": "1.0",
    "title": "hibor_overnight moved +99.3%",
    "publisher": "HK City Pulse",
    "year": 2026,
    "generated_at": "2026-06-21T00:00:00Z",
    "experimental": False,
    "manifest": {
        "cite_version": "1.0",
        "detector": "series_jump",
        "source": "hkma",
        "dataset": "daily-interbank-liquidity",
        "data_sha256": "abc123",
        "generated_at": "2026-06-21T00:00:00Z",
        "threshold": 25.0,
        "runtime_version": "0.1.0",
    },
}

SAMPLE_SIGNAL = {
    "id": "sig:alice:abc",
    "owner": "alice",
    "question": "Tell me when HIBOR spikes",
    "compiled": {
        "source": "hkma",
        "dataset": "daily-figures-interbank-liquidity",
        "detector": "series_jump",
        "field": "hibor_overnight",
        "threshold": 25.0,
        "cadence": "daily",
        "comparison": "period_over_period",
    },
    "channels": [{"kind": "webhook", "target": "https://example.com/hook"}],
    "enabled": True,
    "created_at": "2026-07-01T00:00:00Z",
}

SAMPLE_INVESTIGATION = {
    "id": "inv:series_jump:hkma:abc",
    "seed_insight_id": "series_jump:hkma:abc",
    "seed_source": "hkma",
    "seed_dataset": "daily-interbank-liquidity",
    "seed_title": "hibor_overnight moved +99.3%",
    "title": "Why did HIBOR spike?",
    "owner": "alice",
    "steps": [],
    "notes": [],
    "created_at": "2026-07-01T00:00:00Z",
    "updated_at": "2026-07-01T00:00:00Z",
}


@responses.activate
def test_silence_index() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/silence-index",
        json=SAMPLE_SILENCE_INDEX,
        status=200,
    )
    c = _client()
    idx = c.silence_index("2026-Q2")
    assert idx.score == 75.76
    assert idx.total_events == 25
    assert len(idx.signals) == 1
    assert idx.signals[0].kind == "unattributed_series_jump"
    # period is passed as a query param
    assert "period=2026-Q2" in responses.calls[0].request.url


@responses.activate
def test_unprecedentedness() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/unprecedentedness",
        json=SAMPLE_UNPRECEDENTEDNESS,
        status=200,
    )
    c = _client()
    u = c.unprecedentedness("hkma", "daily-interbank-liquidity", "hibor_overnight", 2.93)
    assert u.value == 2.93
    assert u.percentile == 99.5
    assert u.band is not None
    assert u.band.median == 1.2
    assert u.one_in_n == 200
    assert u.last_exceeded is not None
    assert u.last_exceeded.value == 3.1


@responses.activate
def test_cite_returns_citation_bundle() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/insights/ins1/cite",
        json=SAMPLE_CITATION,
        status=200,
    )
    c = _client()
    cite = c.cite("ins1")
    assert cite.permalink == "/cite/series_jump:hkma:abc"
    assert cite.manifest.detector == "series_jump"
    assert cite.manifest.data_sha256 == "abc123"


@responses.activate
def test_cite_returns_rendered_string_for_format() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/insights/ins1/cite",
        body="@misc{key, title={test}}",
        status=200,
        content_type="text/plain",
    )
    c = _client()
    rendered = c.cite("ins1", fmt="bibtex")
    assert isinstance(rendered, str)
    assert "@misc" in rendered
    # format is passed as a query param
    assert "format=bibtex" in responses.calls[0].request.url


@responses.activate
def test_insights_since_and_lang_params() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/insights",
        json=SAMPLE_INSIGHTS,
        status=200,
    )
    c = _client()
    c.insights(since="2026-07-01T00:00:00Z", lang="zh-HK")
    url = responses.calls[0].request.url
    assert "since=2026-07-01" in url
    assert "lang=zh-HK" in url


@responses.activate
def test_insight_history() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/insights/ins1/history",
        json=SAMPLE_INSIGHTS,
        status=200,
    )
    c = _client()
    history = c.insight_history("ins1")
    assert len(history) == 1
    assert history[0].kind == "series_jump"


@responses.activate
def test_request_and_redeem_auth_token() -> None:
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/auth/request-token",
        json={"token": "tok123", "expires_at": "2026-07-01T00:15:00Z"},
        status=200,
    )
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/auth/redeem",
        json={
            "session_token": "ses456",
            "user": {"id": "u:alice", "email": "alice@example.com", "created_at": "2026-07-01T00:00:00Z"},
        },
        status=200,
    )
    c = _client()
    t = c.request_auth_token("alice@example.com")
    assert t.token == "tok123"
    s = c.redeem_auth_token("tok123")
    assert s.session_token == "ses456"
    assert s.user.email == "alice@example.com"


@responses.activate
def test_auth_me_sends_bearer() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/auth/me",
        json={"id": "u:alice", "email": "alice@example.com", "created_at": "2026-07-01T00:00:00Z"},
        status=200,
    )
    c = _client()
    u = c.auth_me("ses456")
    assert u.email == "alice@example.com"
    assert "Authorization" in responses.calls[0].request.headers
    assert responses.calls[0].request.headers["Authorization"] == "Bearer ses456"


@responses.activate
def test_list_and_create_signals() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/signals",
        json=[SAMPLE_SIGNAL],
        status=200,
    )
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/signals",
        json=SAMPLE_SIGNAL,
        status=200,
    )
    c = _client()
    sigs = c.list_signals(session_token="ses456")
    assert len(sigs) == 1
    assert sigs[0].compiled.detector == "series_jump"
    # Bearer header forwarded
    assert responses.calls[0].request.headers["Authorization"] == "Bearer ses456"

    from hkgov import ScanTarget

    st = ScanTarget(
        source="hkma",
        dataset="daily-figures-interbank-liquidity",
        detector="series_jump",
        field="hibor_overnight",
        threshold=25.0,
    )
    created = c.create_signal(st, question="Tell me when HIBOR spikes", session_token="ses456")
    assert created.id == "sig:alice:abc"


@responses.activate
def test_delete_signal() -> None:
    responses.add(
        responses.DELETE,
        f"{BASE}{PREFIX}/signals/sig1",
        json={"deleted": True},
        status=200,
    )
    c = _client()
    assert c.delete_signal("sig1", session_token="ses456") is True


@responses.activate
def test_preview_signal() -> None:
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/signals/preview",
        json={"findings": SAMPLE_INSIGHTS},
        status=200,
    )
    c = _client()
    from hkgov import ScanTarget

    st = ScanTarget(
        source="hkma",
        dataset="daily-figures-interbank-liquidity",
        detector="series_jump",
        field="hibor_overnight",
        threshold=25.0,
    )
    findings = c.preview_signal(st, session_token="ses456")
    assert len(findings) == 1
    assert findings[0].kind == "series_jump"


@responses.activate
def test_create_and_list_investigations() -> None:
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/investigations",
        json=SAMPLE_INVESTIGATION,
        status=200,
    )
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/investigations",
        json=[SAMPLE_INVESTIGATION],
        status=200,
    )
    c = _client()
    inv = c.create_investigation(
        "series_jump:hkma:abc",
        "hkma",
        "daily-interbank-liquidity",
        "hibor_overnight moved +99.3%",
        title="Why did HIBOR spike?",
        session_token="ses456",
    )
    assert inv.id.startswith("inv:")
    invs = c.list_investigations(session_token="ses456")
    assert len(invs) == 1
    assert invs[0].title == "Why did HIBOR spike?"


@responses.activate
def test_delete_investigation_and_add_note() -> None:
    responses.add(
        responses.DELETE,
        f"{BASE}{PREFIX}/investigations/inv1",
        json={"deleted": True},
        status=200,
    )
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/investigations/inv1/notes",
        json={**SAMPLE_INVESTIGATION, "notes": [{"body": "a note"}]},
        status=200,
    )
    c = _client()
    assert c.delete_investigation("inv1", session_token="ses456") is True
    inv = c.add_investigation_note("inv1", "a note", session_token="ses456")
    assert inv.notes[-1]["body"] == "a note"
    # Verify the request body uses "body" (matching the Rust AddNoteRequest schema)
    sent = responses.calls[-1].request.body
    assert b'"body"' in sent and b'a note' in sent


# ── D-011 remaining: market_players + append_investigation_step ──────────────

SAMPLE_MARKET_PLAYERS = [
    {
        "dept": "HKMA",
        "category": "monetary",
        "players": [
            {"name": "HSBC", "note": "Largest HK bank", "url": "https://hsbc.com.hk"},
            {"name": "BOCHK", "note": "2nd largest"},
        ],
    },
    {
        "dept": "IA",
        "category": "livability",
        "players": [
            {"name": "AIA", "note": "Largest insurer"},
        ],
    },
]


@responses.activate
def test_market_players_no_filter() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/market-players",
        json=SAMPLE_MARKET_PLAYERS,
        status=200,
    )
    c = _client()
    groups = c.market_players()
    assert len(groups) == 2
    assert groups[0].dept == "HKMA"
    assert groups[0].category == "monetary"
    assert len(groups[0].players) == 2
    assert groups[0].players[0].name == "HSBC"
    assert groups[0].players[0].url == "https://hsbc.com.hk"
    assert groups[0].players[1].name == "BOCHK"
    assert groups[0].players[1].url is None
    assert groups[1].dept == "IA"
    assert groups[1].players[0].note == "Largest insurer"


@responses.activate
def test_market_players_filtered_by_dept() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/market-players",
        json=[SAMPLE_MARKET_PLAYERS[0]],
        status=200,
    )
    c = _client()
    groups = c.market_players(dept="HKMA")
    assert len(groups) == 1
    assert groups[0].dept == "HKMA"
    # Verify the dept query param was sent
    assert "dept=HKMA" in responses.calls[-1].request.url


@responses.activate
def test_market_players_filtered_by_category() -> None:
    responses.add(
        responses.GET,
        f"{BASE}{PREFIX}/market-players",
        json=[SAMPLE_MARKET_PLAYERS[1]],
        status=200,
    )
    c = _client()
    groups = c.market_players(category="livability")
    assert len(groups) == 1
    assert groups[0].category == "livability"
    assert "category=livability" in responses.calls[-1].request.url


@responses.activate
def test_append_investigation_step() -> None:
    step_response = {
        **SAMPLE_INVESTIGATION,
        "steps": [
            {
                "id": "s1",
                "kind": "qa",
                "prompt": "Why did HIBOR spike?",
                "answer": {
                    "text": "Liquidity tightened",
                    "confidence": 0.8,
                    "trace": [
                        {"tool": "query_dataset", "arguments": {"x": 1}, "result": [1, 2]}
                    ],
                },
                "trace": [
                    {"tool": "run_detector", "arguments": {"d": "x"}, "result": {}}
                ],
                "executed_at": "2026-07-13T00:00:00Z",
                "annotation": None,
            }
        ],
    }
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/investigations/inv1/steps",
        json=step_response,
        status=200,
    )
    c = _client()
    inv = c.append_investigation_step(
        "inv1",
        kind="qa",
        prompt="Why did HIBOR spike?",
        session_token="ses456",
    )
    assert inv.id.startswith("inv:")
    assert len(inv.steps) == 1
    assert inv.steps[0].kind == "qa"
    assert inv.steps[0].prompt == "Why did HIBOR spike?"
    # answer + trace + executed_at must be parsed (regression: previously dropped)
    assert inv.steps[0].answer is not None
    assert inv.steps[0].answer.text == "Liquidity tightened"
    assert inv.steps[0].answer.confidence == 0.8
    assert len(inv.steps[0].answer.trace) == 1
    assert inv.steps[0].answer.trace[0].tool == "query_dataset"
    assert len(inv.steps[0].trace) == 1
    assert inv.steps[0].trace[0].tool == "run_detector"
    assert inv.steps[0].executed_at == "2026-07-13T00:00:00Z"
    # Verify the request body has the right shape
    sent = responses.calls[0].request.body
    assert b'"kind"' in sent and b'qa' in sent
    assert b'"prompt"' in sent
    # Verify the bearer token was sent
    assert responses.calls[0].request.headers["Authorization"] == "Bearer ses456"


@responses.activate
def test_append_investigation_step_chip_with_annotation() -> None:
    step_response = {
        **SAMPLE_INVESTIGATION,
        "steps": [
            {
                "id": "s1",
                "kind": "chip",
                "prompt": "query_dataset",
                "trace": [],
                "executed_at": "2026-07-13T00:00:00Z",
                "annotation": "checked liquidity",
            }
        ],
    }
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/investigations/inv1/steps",
        json=step_response,
        status=200,
    )
    c = _client()
    inv = c.append_investigation_step(
        "inv1",
        kind="chip",
        prompt="query_dataset",
        annotation="checked liquidity",
        session_token="ses456",
    )
    assert inv.steps[0].annotation == "checked liquidity"
    sent = responses.calls[0].request.body
    assert b'"annotation"' in sent and b'checked liquidity' in sent


@responses.activate
def test_append_investigation_step_serializes_trace() -> None:
    """Passing a TraceStep list (e.g. from ask()) must serialize to JSON —
    regression: dataclasses aren't JSON-serializable by default."""
    step_response = {**SAMPLE_INVESTIGATION, "steps": []}
    responses.add(
        responses.POST,
        f"{BASE}{PREFIX}/investigations/inv1/steps",
        json=step_response,
        status=200,
    )
    c = _client()
    trace = [TraceStep(tool="query_dataset", arguments={"a": 1}, result=[1, 2])]
    ans = Answer(text="ok", confidence=0.5, trace=list(trace))
    c.append_investigation_step(
        "inv1", kind="qa", prompt="p", answer=ans, trace=trace
    )
    sent = responses.calls[0].request.body
    # If serialization had failed, requests would have raised before this line.
    assert b'"tool": "query_dataset"' in sent
    assert b'"arguments": {"a": 1}' in sent


@responses.activate
def test_update_signal() -> None:
    updated = {
        **SAMPLE_SIGNAL,
        "question": "renamed",
        "enabled": False,
    }
    responses.add(
        responses.PATCH,
        f"{BASE}{PREFIX}/signals/sig1",
        json=updated,
        status=200,
    )
    c = _client()
    sig = c.update_signal("sig1", question="renamed", enabled=False, session_token="ses456")
    assert sig.question == "renamed"
    assert sig.enabled is False
    # PATCH verb + only the changed fields are sent.
    assert responses.calls[0].request.method == "PATCH"
    sent = responses.calls[0].request.body
    assert b'"question": "renamed"' in sent
    assert b'"enabled": false' in sent
    assert responses.calls[0].request.headers["Authorization"] == "Bearer ses456"


def test_update_signal_requires_a_field() -> None:
    c = _client()
    with pytest.raises(ValueError):
        c.update_signal("sig1")
