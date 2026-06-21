"""Synchronous HTTP client for hkgov-rethink."""

from __future__ import annotations

from typing import Any, Optional

import requests

from .models import (
    AlertLogEntry,
    Answer,
    CategoryGroup,
    DatasetMeta,
    EvidenceRef,
    Health,
    Insight,
    Record,
    RecordPage,
    SourceHealth,
    TraceStep,
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

    def _get(self, path: str, params: Optional[dict[str, Any]] = None) -> Any:
        try:
            r = requests.get(
                self._url(path), headers=self._headers, params=params, timeout=self._timeout
            )
        except requests.RequestException as e:
            raise HkGovError(f"transport error: {e}") from e
        return self._json(r)

    def _post(self, path: str, body: dict[str, Any]) -> Any:
        headers = {"Content-Type": "application/json", **self._headers}
        try:
            r = requests.post(
                self._url(path), headers=headers, json=body, timeout=self._timeout
            )
        except requests.RequestException as e:
            raise HkGovError(f"transport error: {e}") from e
        return self._json(r)

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

    def insights(self, limit: int = 20) -> list[Insight]:
        d = self._get("/insights", params={"limit": limit})
        return [self._insight(x) for x in d]

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
        return Answer(
            text=d.get("text", ""),
            confidence=float(d.get("confidence", 0.0)),
            trace=[
                TraceStep(tool=s["tool"], arguments=s.get("arguments"), result=s.get("result"))
                for s in d.get("trace", [])
            ],
        )

    # ---- helpers --------------------------------------------------------------

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
