"""Synchronous HTTP client for hkgov-rethink."""

from __future__ import annotations

from typing import Any, Optional

import requests

from .models import (
    AlertLogEntry,
    Answer,
    Brief,
    BriefItem,
    CategoryGroup,
    Citation,
    DatasetMeta,
    EvidenceRef,
    Finding,
    Health,
    Insight,
    Investigation,
    InvestigationStep,
    LastExceeded,
    MarketPlayerGroup,
    NormalRange,
    PlayerEntry,
    Record,
    RecordPage,
    ReproducibilityManifest,
    ScanTarget,
    Session,
    SilenceIndex,
    SilenceSignal,
    Signal,
    SignalChannel,
    SourceHealth,
    TokenResponse,
    TraceStep,
    Unprecedentedness,
    User,
)


class HkGovError(Exception):
    """Raised when the API returns a non-2xx response or transport fails."""


class HkGov:
    """Client for a running hkgov-rethink server.

    Args:
        base_url: Scheme + host + port, e.g. ``http://localhost:8080``.
        api_key: Optional ``X-API-Key`` value, required when the server has
            ``api.api_key`` set.
        prefix: API version prefix, default ``/v1``.
        timeout: Per-request timeout in seconds.
    """

    def __init__(
        self,
        base_url: str = "http://localhost:8080",
        api_key: Optional[str] = None,
        prefix: str = "/v1",
        timeout: float = 30.0,
    ) -> None:
        self._base = base_url.rstrip("/")
        self._prefix = prefix.strip("/")
        self._timeout = timeout
        self._headers: dict[str, str] = {}
        if api_key:
            self._headers["X-API-Key"] = api_key

    # ---- low-level ------------------------------------------------------------

    def _url(self, path: str) -> str:
        path = path.lstrip("/")
        if self._prefix:
            return f"{self._base}/{self._prefix}/{path}"
        return f"{self._base}/{path}"

    def _get(
        self,
        path: str,
        params: Optional[dict[str, Any]] = None,
        headers: Optional[dict[str, str]] = None,
    ) -> Any:
        h = {**self._headers, **(headers or {})}
        try:
            r = requests.get(
                self._url(path), headers=h, params=params, timeout=self._timeout
            )
        except requests.RequestException as e:
            raise HkGovError(f"transport error: {e}") from e
        return self._json(r)

    def _post(
        self,
        path: str,
        body: dict[str, Any],
        headers: Optional[dict[str, str]] = None,
    ) -> Any:
        h = {"Content-Type": "application/json", **self._headers, **(headers or {})}
        try:
            r = requests.post(
                self._url(path), headers=h, json=body, timeout=self._timeout
            )
        except requests.RequestException as e:
            raise HkGovError(f"transport error: {e}") from e
        return self._json(r)

    def _patch(
        self,
        path: str,
        body: dict[str, Any],
        headers: Optional[dict[str, str]] = None,
    ) -> Any:
        h = {"Content-Type": "application/json", **self._headers, **(headers or {})}
        try:
            r = requests.patch(
                self._url(path), headers=h, json=body, timeout=self._timeout
            )
        except requests.RequestException as e:
            raise HkGovError(f"transport error: {e}") from e
        return self._json(r)

    def _delete(
        self,
        path: str,
        headers: Optional[dict[str, str]] = None,
    ) -> Any:
        """Issue a DELETE and return the parsed JSON body."""
        h = {**self._headers, **(headers or {})}
        try:
            r = requests.delete(self._url(path), headers=h, timeout=self._timeout)
        except requests.RequestException as e:
            raise HkGovError(f"transport error: {e}") from e
        return self._json(r)

    def _auth_headers(self, session_token: Optional[str]) -> Optional[dict[str, str]]:
        """Build the Authorization header for a session token, or None."""
        if not session_token:
            return None
        return {"Authorization": f"Bearer {session_token}"}

    @staticmethod
    def _json(r: requests.Response) -> Any:
        if not r.ok:
            try:
                detail = r.json()
            except ValueError:
                detail = r.text
            raise HkGovError(f"{r.status_code}: {detail}")
        return r.json()

    # ---- endpoints ------------------------------------------------------------

    def health(self) -> Health:
        d = self._raw_get("/health")
        return Health(status=d["status"], version=d["version"])

    def _raw_get(self, path: str) -> Any:
        # For endpoints that live at root rather than under the version prefix
        # (e.g. /health for LB probes).
        try:
            r = requests.get(
                f"{self._base}{path}", headers=self._headers, timeout=self._timeout
            )
        except requests.RequestException as e:
            raise HkGovError(f"transport error: {e}") from e
        return self._json(r)

    def source_health(self) -> list[SourceHealth]:
        d = self._get("/health/sources")
        return [SourceHealth(source=x["source"], circuit=x["circuit"]) for x in d]

    def sources(
        self,
        *,
        source: Optional[str] = None,
        category: Optional[str] = None,
        tag: Optional[list[str] | str] = None,
        cadence: Optional[str] = None,
        q: Optional[str] = None,
    ) -> list[DatasetMeta]:
        """List ingested datasets. All filters optional; compose with AND.

        - ``category`` — one of monetary/fiscal/property/trade/population/
          livability/government/other.
        - ``tag`` — a single tag or a list; matches if the dataset has ANY.
        - ``cadence`` — daily/weekly/monthly/quarterly/biannual/annual/unknown.
        - ``q`` — case-insensitive substring over title+description+id.
        """
        params: dict[str, Any] = {}
        if source:
            params["source"] = source
        if category:
            params["category"] = category
        if cadence:
            params["cadence"] = cadence
        if q:
            params["q"] = q
        if tag:
            # Allow either a single string or a list; the API takes repeated params.
            tags = [tag] if isinstance(tag, str) else list(tag)
            params["tag"] = tags
        d = self._get("/sources", params=params or None)
        return [self._meta(x) for x in d]

    def categories(self) -> list[CategoryGroup]:
        """The browse entry point: every domain category with its dataset count."""
        d = self._get("/categories")
        return [
            CategoryGroup(category=x["category"], count=x["count"], datasets=x["datasets"])
            for x in d
        ]

    def dataset(self, source: str, dataset: str) -> Optional[DatasetMeta]:
        d = self._get(f"/datasets/{source}/{dataset}")
        return self._meta(d) if d else None

    def records(
        self, source: str, dataset: str, offset: int = 0, limit: int = 100
    ) -> RecordPage:
        d = self._get(
            f"/datasets/{source}/{dataset}/records",
            params={"offset": offset, "limit": limit},
        )
        return RecordPage(
            source=d["source"],
            dataset=d["dataset"],
            total=d["total"],
            offset=d["offset"],
            limit=d["limit"],
            records=[Record(record_id=r["record_id"], fields=r.get("fields", {})) for r in d["records"]],
        )

    def insights(
        self,
        limit: int = 20,
        *,
        since: Optional[str] = None,
        lang: Optional[str] = None,
    ) -> list[Insight]:
        """AI-agent generated insights with evidence.

        Args:
            limit: Maximum number of insights to return.
            since: Only insights generated after this timestamp. Accepts RFC 3339
                (``2026-07-01T00:00:00Z``) or epoch seconds (``1751328000``).
                Bad values raise ``HkGovError`` (400) — they do NOT silently fall
                through to the unfiltered list.
            lang: ``"zh-HK"`` for deterministic Traditional-Chinese summaries.
        """
        params: dict[str, Any] = {"limit": limit}
        if since:
            params["since"] = since
        if lang:
            params["lang"] = lang
        d = self._get("/insights", params=params)
        return [self._insight(x) for x in d]

    def brief(self, limit: int = 5) -> Brief:
        """The ranked daily brief — the top items worth knowing about today.

        Items flatten the insight fields onto themselves (mirroring the API's
        ``#[serde(flatten)]``); this method re-nests them under ``.insight`` so
        each ``BriefItem`` exposes both ``rank``/``score`` and the full Insight.
        """
        d = self._get("/brief", params={"limit": limit})
        items = [
            BriefItem(
                rank=x["rank"],
                score=float(x["score"]),
                insight=self._insight(x),
            )
            for x in d.get("items", [])
        ]
        return Brief(generated_at=d.get("generated_at", ""), items=items)

    def feedback(self, insight_id: str, useful: bool, note: Optional[str] = None) -> None:
        """Record was-this-useful feedback on an insight (the success metric).

        Args:
            insight_id: The insight id (URL-encoded for you).
            useful: True = useful, False = not useful.
            note: Optional free-text reason (especially for "not useful").
        """
        body: dict[str, Any] = {"useful": useful}
        if note is not None:
            body["note"] = note
        self._post(f"/insights/{insight_id}/feedback", body)

    def feedback_score(self, insight_id: str) -> int:
        """Net usefulness (up − down) for an insight."""
        d = self._get(f"/insights/{insight_id}/feedback")
        return int(d.get("net_useful", 0))

    def alerts(self, limit: int = 20) -> list[AlertLogEntry]:
        d = self._get("/alerts", params={"limit": limit})
        return [
            AlertLogEntry(
                insight_id=x["insight_id"],
                insight_kind=x["insight_kind"],
                severity=x["severity"],
                sink=x["sink"],
                status=x["status"],
                dispatched_at=x["dispatched_at"],
            )
            for x in d
        ]

    def ask(self, question: str) -> Answer:
        d = self._post("/ask", {"question": question})
        return self._answer(d)

    # ---- v7 product surface: Silence Index + Unprecedentedness + Cite-It ------

    def silence_index(
        self, period: Optional[str] = None, *, source: Optional[str] = None
    ) -> SilenceIndex:
        """The HKMA Silence Index — a 0–100 opacity score for a period.

        Args:
            period: Period key like ``"2026-Q2"``. Omit for the full held corpus.
            source: Scope to a different source (default HKMA for backward compat).
        """
        params: dict[str, Any] = {}
        if period:
            params["period"] = period
        if source:
            params["source"] = source
        d = self._get("/silence-index", params=params or None)
        return self._silence_index(d)

    def unprecedentedness(
        self,
        source: str,
        dataset: str,
        field: str,
        value: float,
        *,
        k: Optional[float] = None,
    ) -> Unprecedentedness:
        """Score a value against its stored history (percentile, band, 1-in-N).

        Args:
            source: e.g. ``"hkma"``.
            dataset: e.g. ``"daily-figures-interbank-liquidity"``.
            field: The numeric field whose history defines "normal".
            value: The current observation to score.
            k: Optional band multiplier (defaults to 3.5).
        """
        params: dict[str, Any] = {
            "source": source,
            "dataset": dataset,
            "field": field,
            "value": value,
        }
        if k is not None:
            params["k"] = k
        d = self._get("/unprecedentedness", params=params)
        return self._unprecedentedness(d)

    def cite(
        self,
        insight_id: str,
        *,
        fmt: Optional[str] = None,
        base_url: Optional[str] = None,
    ) -> Any:
        """Get a citation for an insight.

        Args:
            insight_id: The insight id.
            fmt: One of ``bibtex``, ``ris``, ``apa``, ``chicago``, ``markdown``.
                When given, returns the rendered citation **string**. When
                omitted, returns the full :class:`Citation` bundle (with the
                reproducibility manifest).
            base_url: The public origin for the permalink (defaults to
                ``http://localhost:8080`` if the server doesn't know better).

        Returns:
            A :class:`Citation` when ``fmt`` is None, or a ``str`` when a format
            is requested.
        """
        params: dict[str, Any] = {}
        if fmt:
            params["format"] = fmt
        if base_url:
            params["base_url"] = base_url
        if fmt:
            # Rendered format comes back as text/plain (not JSON).
            try:
                r = requests.get(
                    self._url(f"/insights/{insight_id}/cite"),
                    headers=self._headers,
                    params=params,
                    timeout=self._timeout,
                )
            except requests.RequestException as e:
                raise HkGovError(f"transport error: {e}") from e
            if not r.ok:
                raise HkGovError(f"{r.status_code}: {r.text}")
            return r.text
        d = self._get(f"/insights/{insight_id}/cite", params=params or None)
        return self._citation(d)

    # ---- v8 product surface: Signals + Investigations + Identity --------------

    def request_auth_token(self, email: str) -> TokenResponse:
        """Request a magic-link auth token for an email.

        The token is delivered out-of-band (email) unless the server runs with
        ``dev_return_auth_token=true``, in which case it's in the response body.
        """
        d = self._post("/auth/request-token", {"email": email})
        return TokenResponse(token=d.get("token"), expires_at=d.get("expires_at", ""))

    def redeem_auth_token(self, token: str) -> Session:
        """Exchange a magic-link token for a session."""
        d = self._post("/auth/redeem", {"token": token})
        return Session(
            session_token=d["session_token"],
            user=self._user(d["user"]),
        )

    def auth_me(self, session_token: str) -> User:
        """Resolve a session token to the current user.

        Raises ``HkGovError`` (401) if the session is invalid/expired.
        """
        d = self._get(
            "/auth/me",
            headers={"Authorization": f"Bearer {session_token}"},
        )
        return self._user(d)

    def list_signals(self, *, limit: int = 20, session_token: Optional[str] = None) -> list[Signal]:
        """List the authenticated caller's signals (ownership-scoped)."""
        headers = self._auth_headers(session_token)
        d = self._get("/signals", params={"limit": limit}, headers=headers)
        return [self._signal(x) for x in d]

    def create_signal(
        self,
        compiled: ScanTarget,
        *,
        question: Optional[str] = None,
        channels: Optional[list[SignalChannel]] = None,
        session_token: Optional[str] = None,
    ) -> Signal:
        """Create a signal subscription."""
        body: dict[str, Any] = {"compiled": self._scan_target_to_dict(compiled)}
        if question:
            body["question"] = question
        if channels:
            body["channels"] = [{"kind": c.kind, **({"target": c.target} if c.target else {})} for c in channels]
        headers = self._auth_headers(session_token)
        d = self._post("/signals", body, headers=headers)
        return self._signal(d)

    def get_signal(self, signal_id: str, *, session_token: Optional[str] = None) -> Optional[Signal]:
        headers = self._auth_headers(session_token)
        d = self._get(f"/signals/{signal_id}", headers=headers)
        return self._signal(d) if d else None

    def update_signal(
        self,
        signal_id: str,
        *,
        question: Optional[str] = None,
        compiled: Optional[ScanTarget] = None,
        channels: Optional[list[SignalChannel]] = None,
        enabled: Optional[bool] = None,
        session_token: Optional[str] = None,
    ) -> Signal:
        """Partially update a signal subscription (PATCH).

        Only the fields you pass are sent; omitted fields are left unchanged
        on the server (mirrors the all-optional ``SignalPatch`` body). At
        least one field must be provided.
        """
        body: dict[str, Any] = {}
        if question is not None:
            body["question"] = question
        if compiled is not None:
            body["compiled"] = self._scan_target_to_dict(compiled)
        if channels is not None:
            body["channels"] = [
                {"kind": c.kind, **({"target": c.target} if c.target else {})} for c in channels
            ]
        if enabled is not None:
            body["enabled"] = enabled
        if not body:
            raise ValueError("update_signal requires at least one field to update")
        headers = self._auth_headers(session_token)
        d = self._patch(f"/signals/{signal_id}", body, headers=headers)
        return self._signal(d)

    def delete_signal(self, signal_id: str, *, session_token: Optional[str] = None) -> bool:
        headers = self._auth_headers(session_token)
        d = self._delete(f"/signals/{signal_id}", headers=headers)
        return bool(d.get("deleted", False))

    def preview_signal(
        self, compiled: ScanTarget, *, window_days: int = 90, session_token: Optional[str] = None
    ) -> list[Finding]:
        """Preview what a scan target would have fired — preview IS production."""
        body = {
            "compiled": self._scan_target_to_dict(compiled),
            "window_days": window_days,
        }
        headers = self._auth_headers(session_token)
        d = self._post("/signals/preview", body, headers=headers)
        return [self._finding(x) for x in d.get("findings", [])]

    def list_investigations(
        self, *, limit: int = 20, session_token: Optional[str] = None
    ) -> list[Investigation]:
        headers = self._auth_headers(session_token)
        d = self._get("/investigations", params={"limit": limit}, headers=headers)
        return [self._investigation(x) for x in d]

    def create_investigation(
        self,
        seed_insight_id: str,
        seed_source: str,
        seed_dataset: str,
        seed_title: str,
        *,
        title: Optional[str] = None,
        session_token: Optional[str] = None,
    ) -> Investigation:
        """Create a drill-in investigation from an insight."""
        body: dict[str, Any] = {
            "seed_insight_id": seed_insight_id,
            "seed_source": seed_source,
            "seed_dataset": seed_dataset,
            "seed_title": seed_title,
        }
        if title:
            body["title"] = title
        headers = self._auth_headers(session_token)
        d = self._post("/investigations", body, headers=headers)
        return self._investigation(d)

    def get_investigation(
        self, inv_id: str, *, session_token: Optional[str] = None
    ) -> Optional[Investigation]:
        headers = self._auth_headers(session_token)
        d = self._get(f"/investigations/{inv_id}", headers=headers)
        return self._investigation(d) if d else None

    def delete_investigation(self, inv_id: str, *, session_token: Optional[str] = None) -> bool:
        headers = self._auth_headers(session_token)
        d = self._delete(f"/investigations/{inv_id}", headers=headers)
        return bool(d.get("deleted", False))

    def add_investigation_note(
        self, inv_id: str, text: str, *, session_token: Optional[str] = None
    ) -> Investigation:
        """Add a free-text note to an investigation."""
        headers = self._auth_headers(session_token)
        d = self._post(f"/investigations/{inv_id}/notes", {"body": text}, headers=headers)
        return self._investigation(d)

    def append_investigation_step(
        self,
        inv_id: str,
        kind: str,
        prompt: str,
        *,
        answer: Optional[Answer] = None,
        trace: Optional[list[TraceStep]] = None,
        annotation: Optional[str] = None,
        session_token: Optional[str] = None,
    ) -> Investigation:
        """Append a step (chip/qa/finding_promotion) to an investigation.

        This is the agent-driven step endpoint: for ``kind="qa"`` the server
        runs ``run_agent_loop`` against the investigation's seed insight and
        appends the result. For ``kind="chip"`` it's a one-click preset tool
        call. Returns the updated investigation with the new step appended.
        """
        body: dict[str, Any] = {"kind": kind, "prompt": prompt}
        if answer is not None:
            body["answer"] = {
                "text": answer.text,
                "confidence": answer.confidence,
                "trace": [self._trace_to_dict(t) for t in answer.trace],
            }
        if trace is not None:
            body["trace"] = [self._trace_to_dict(t) for t in trace]
        if annotation is not None:
            body["annotation"] = annotation
        headers = self._auth_headers(session_token)
        d = self._post(f"/investigations/{inv_id}/steps", body, headers=headers)
        return self._investigation(d)

    def insight_history(self, insight_id: str) -> list[Insight]:
        """Prior versions of an insight (the evolution history)."""
        d = self._get(f"/insights/{insight_id}/history")
        return [self._insight(x) for x in d]

    def market_players(
        self, *, dept: Optional[str] = None, category: Optional[str] = None
    ) -> list[MarketPlayerGroup]:
        """The curated related-market-players directory.

        Filter by ``dept`` (e.g. ``"HKMA"``) or ``category`` (e.g.
        ``"monetary"``). With no filters, returns every department group.
        """
        params: dict[str, Any] = {}
        if dept:
            params["dept"] = dept
        if category:
            params["category"] = category
        d = self._get("/market-players", params=params or None)
        return [self._market_player_group(x) for x in d]

    # ---- helpers --------------------------------------------------------------

    @staticmethod
    def _player_entry(x: dict[str, Any]) -> PlayerEntry:
        return PlayerEntry(
            name=x.get("name", ""),
            note=x.get("note", ""),
            url=x.get("url"),
        )

    @staticmethod
    def _market_player_group(x: dict[str, Any]) -> MarketPlayerGroup:
        return MarketPlayerGroup(
            dept=x.get("dept", ""),
            category=x.get("category", "other"),
            players=[HkGov._player_entry(p) for p in x.get("players", [])],
        )

    @staticmethod
    def _meta(x: dict[str, Any]) -> DatasetMeta:
        return DatasetMeta(
            source=x["source"],
            dataset=x["dataset"],
            title=x.get("title", ""),
            description=x.get("description"),
            category=x.get("category", "other"),
            tags=x.get("tags", []),
            cadence=x.get("cadence", "unknown"),
            refresh_interval_secs=x.get("refresh_interval_secs", 0),
            last_refreshed_at=x.get("last_refreshed_at"),
            record_count=x.get("record_count", 0),
        )

    @staticmethod
    def _insight(x: dict[str, Any]) -> Insight:
        return Insight(
            id=x["id"],
            kind=x["kind"],
            severity=x["severity"],
            title=x["title"],
            summary=x.get("summary", ""),
            source=x.get("source", ""),
            dataset=x.get("dataset", ""),
            evidence=[
                EvidenceRef(
                    record_id=e["record_id"],
                    field=e["field"],
                    value=e.get("value"),
                    context=e.get("context"),
                )
                for e in x.get("evidence", [])
            ],
            confidence=float(x.get("confidence", 0.0)),
            generated_at=x.get("generated_at", ""),
            producer=x.get("producer", ""),
        )

    # ---- v7/v8 factory helpers ------------------------------------------------

    @staticmethod
    def _user(x: dict[str, Any]) -> User:
        return User(
            id=x["id"],
            email=x["email"],
            created_at=x.get("created_at", ""),
        )

    @staticmethod
    def _silence_index(d: dict[str, Any]) -> SilenceIndex:
        return SilenceIndex(
            label=d.get("label", ""),
            methodology_version=d.get("methodology_version", ""),
            source=d.get("source", ""),
            period=d.get("period", ""),
            score=float(d.get("score", 0.0)),
            raw_score=float(d.get("raw_score", 0.0)),
            computed_at=d.get("computed_at", ""),
            total_events=int(d.get("total_events", 0)),
            signals=[
                SilenceSignal(
                    kind=s.get("kind", ""),
                    count=int(s.get("count", 0)),
                    weight=float(s.get("weight", 0.0)),
                    contribution=float(s.get("contribution", 0.0)),
                    evidence_ids=s.get("evidence_ids", []),
                )
                for s in d.get("signals", [])
            ],
        )

    @staticmethod
    def _unprecedentedness(d: dict[str, Any]) -> Unprecedentedness:
        band_raw = d.get("band")
        band = (
            NormalRange(
                low=float(band_raw["low"]),
                high=float(band_raw["high"]),
                median=float(band_raw["median"]),
                mad=float(band_raw["mad"]),
            )
            if band_raw
            else None
        )
        le_raw = d.get("last_exceeded")
        last_exceeded = (
            LastExceeded(
                record_id=le_raw["record_id"],
                value=float(le_raw["value"]),
                when=le_raw.get("when"),
                pct_beyond_edge=float(le_raw.get("pct_beyond_edge", 0.0)),
            )
            if le_raw
            else None
        )
        pct = d.get("percentile")
        return Unprecedentedness(
            value=float(d.get("value", 0.0)),
            n=int(d.get("n", 0)),
            percentile=float(pct) if pct is not None else None,
            band=band,
            one_in_n=d.get("one_in_n"),
            hist_min=d.get("hist_min"),
            hist_max=d.get("hist_max"),
            last_exceeded=last_exceeded,
        )

    @staticmethod
    def _citation(d: dict[str, Any]) -> Citation:
        m = d.get("manifest", {})
        return Citation(
            permalink=d.get("permalink", ""),
            insight_id=d.get("insight_id", ""),
            cite_version=d.get("cite_version", ""),
            title=d.get("title", ""),
            publisher=d.get("publisher", ""),
            year=int(d.get("year", 0)),
            generated_at=d.get("generated_at", ""),
            experimental=bool(d.get("experimental", False)),
            manifest=ReproducibilityManifest(
                cite_version=m.get("cite_version", ""),
                detector=m.get("detector", ""),
                source=m.get("source", ""),
                dataset=m.get("dataset", ""),
                data_sha256=m.get("data_sha256", ""),
                generated_at=m.get("generated_at", ""),
                threshold=m.get("threshold"),
                runtime_version=m.get("runtime_version"),
            ),
        )

    @staticmethod
    def _scan_target(x: dict[str, Any]) -> ScanTarget:
        return ScanTarget(
            source=x.get("source", ""),
            dataset=x.get("dataset", ""),
            detector=x.get("detector", ""),
            field=x.get("field"),
            threshold=x.get("threshold"),
            field_b=x.get("field_b"),
            companion=x.get("companion"),
            cadence=x.get("cadence", "unknown"),
            comparison=x.get("comparison", "period_over_period"),
            direction=x.get("direction"),
        )

    @staticmethod
    def _scan_target_to_dict(t: ScanTarget) -> dict[str, Any]:
        d: dict[str, Any] = {
            "source": t.source,
            "dataset": t.dataset,
            "detector": t.detector,
            "cadence": t.cadence,
            "comparison": t.comparison,
        }
        if t.field:
            d["field"] = t.field
        if t.threshold is not None:
            d["threshold"] = t.threshold
        if t.field_b:
            d["field_b"] = t.field_b
        if t.companion:
            d["companion"] = t.companion
        if t.direction:
            d["direction"] = t.direction
        return d

    @classmethod
    def _signal(cls, x: dict[str, Any]) -> Signal:
        return Signal(
            id=x["id"],
            owner=x.get("owner", ""),
            question=x.get("question", ""),
            compiled=cls._scan_target(x.get("compiled", {})),
            enabled=bool(x.get("enabled", True)),
            created_at=x.get("created_at", ""),
            updated_at=x.get("updated_at"),
            channels=[
                SignalChannel(kind=c.get("kind", ""), target=c.get("target"))
                for c in x.get("channels", [])
            ],
        )

    @classmethod
    def _investigation(cls, x: dict[str, Any]) -> Investigation:
        return Investigation(
            id=x["id"],
            seed_insight_id=x.get("seed_insight_id", ""),
            seed_source=x.get("seed_source", ""),
            seed_dataset=x.get("seed_dataset", ""),
            seed_title=x.get("seed_title", ""),
            title=x.get("title", ""),
            owner=x.get("owner", ""),
            steps=[
                InvestigationStep(
                    id=s.get("id", ""),
                    kind=s.get("kind", ""),
                    prompt=s.get("prompt", ""),
                    answer=cls._answer(s["answer"]) if s.get("answer") else None,
                    trace=cls._trace(s.get("trace", [])),
                    annotation=s.get("annotation"),
                    executed_at=s.get("executed_at"),
                )
                for s in x.get("steps", [])
            ],
            notes=x.get("notes", []),
            created_at=x.get("created_at", ""),
            updated_at=x.get("updated_at", ""),
        )

    @staticmethod
    def _trace(items: Any) -> list[TraceStep]:
        return [
            TraceStep(tool=s["tool"], arguments=s.get("arguments"), result=s.get("result"))
            for s in items or []
        ]

    @staticmethod
    def _trace_to_dict(t: TraceStep) -> dict[str, Any]:
        """Serialize a TraceStep for the request body (dataclasses aren't JSON-
        serializable by default, so callers passing a trace from `ask()` would
        otherwise hit TypeError at request time)."""
        return {"tool": t.tool, "arguments": t.arguments, "result": t.result}

    @classmethod
    def _answer(cls, x: dict[str, Any]) -> Answer:
        return Answer(
            text=x.get("text", ""),
            confidence=float(x.get("confidence", 0.0)),
            trace=cls._trace(x.get("trace", [])),
        )

    @staticmethod
    def _finding(x: dict[str, Any]) -> Finding:
        return Finding(
            kind=x.get("kind", ""),
            source=x.get("source", ""),
            dataset=x.get("dataset", ""),
            title=x.get("title", ""),
            summary=x.get("summary", ""),
            severity=x.get("severity", "info"),
            confidence=float(x.get("confidence", 0.0)),
            evidence=[
                EvidenceRef(
                    record_id=e["record_id"],
                    field=e["field"],
                    value=e.get("value"),
                    context=e.get("context"),
                )
                for e in x.get("evidence", [])
            ],
        )
