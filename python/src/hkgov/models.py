"""Typed dataclasses mirroring the hkgov-rethink HTTP API response shapes."""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Optional


@dataclass(frozen=True)
class Health:
    status: str
    version: str


@dataclass(frozen=True)
class SourceHealth:
    source: str
    circuit: str


@dataclass(frozen=True)
class DatasetMeta:
    source: str
    dataset: str
    title: str
    description: Optional[str]
    category: str
    tags: list[str]
    cadence: str
    refresh_interval_secs: int
    last_refreshed_at: Optional[str]
    record_count: int


@dataclass(frozen=True)
class CategoryGroup:
    category: str
    count: int
    datasets: list[str]


@dataclass(frozen=True)
class Record:
    record_id: str
    fields: dict[str, Any]


@dataclass(frozen=True)
class RecordPage:
    source: str
    dataset: str
    total: int
    offset: int
    limit: int
    records: list[Record]


@dataclass(frozen=True)
class EvidenceRef:
    record_id: str
    field: str
    value: Any
    context: Optional[str]


@dataclass(frozen=True)
class Insight:
    id: str
    kind: str
    severity: str
    title: str
    summary: str
    source: str
    dataset: str
    evidence: list[EvidenceRef]
    confidence: float
    generated_at: str
    producer: str
    experimental: bool = False


@dataclass(frozen=True)
class BriefItem:
    rank: int
    score: float
    # The insight fields are flattened into the item (serde flatten), so they
    # are spread onto BriefItem itself via the client factory.
    insight: Insight


@dataclass(frozen=True)
class Brief:
    generated_at: str
    items: list[BriefItem]


@dataclass(frozen=True)
class TraceStep:
    tool: str
    arguments: Any
    result: Any


@dataclass(frozen=True)
class Answer:
    text: str
    confidence: float
    trace: list[TraceStep] = field(default_factory=list)


@dataclass(frozen=True)
class AlertLogEntry:
    insight_id: str
    insight_kind: str
    severity: str
    sink: str
    status: str
    dispatched_at: str


# ---- v7/v8 product-layer types ---------------------------------------------


@dataclass(frozen=True)
class SilenceSignal:
    kind: str
    count: int
    weight: float
    contribution: float
    evidence_ids: list[str]


@dataclass(frozen=True)
class SilenceIndex:
    label: str
    methodology_version: str
    source: str
    period: str
    score: float
    raw_score: float
    computed_at: str
    total_events: int
    signals: list[SilenceSignal] = field(default_factory=list)


@dataclass(frozen=True)
class NormalRange:
    low: float
    high: float
    median: float
    mad: float


@dataclass(frozen=True)
class LastExceeded:
    record_id: str
    value: float
    when: Optional[str]
    pct_beyond_edge: float


@dataclass(frozen=True)
class Unprecedentedness:
    value: float
    n: int
    percentile: Optional[float] = None
    band: Optional[NormalRange] = None
    one_in_n: Optional[int] = None
    hist_min: Optional[float] = None
    hist_max: Optional[float] = None
    last_exceeded: Optional[LastExceeded] = None


@dataclass(frozen=True)
class ReproducibilityManifest:
    cite_version: str
    detector: str
    source: str
    dataset: str
    data_sha256: str
    generated_at: str
    threshold: Optional[float] = None
    runtime_version: Optional[str] = None


@dataclass(frozen=True)
class Citation:
    permalink: str
    insight_id: str
    cite_version: str
    title: str
    publisher: str
    year: int
    generated_at: str
    experimental: bool
    manifest: ReproducibilityManifest


@dataclass(frozen=True)
class ScanTarget:
    source: str
    dataset: str
    detector: str
    field: Optional[str] = None
    threshold: Optional[float] = None
    field_b: Optional[str] = None
    companion: Optional[dict[str, Any]] = None
    cadence: str = "unknown"
    comparison: str = "period_over_period"
    direction: Optional[str] = None


@dataclass(frozen=True)
class SignalChannel:
    kind: str
    target: Optional[str] = None


@dataclass(frozen=True)
class Signal:
    id: str
    owner: str
    question: str
    compiled: ScanTarget
    enabled: bool
    created_at: str
    updated_at: Optional[str] = None
    channels: list[SignalChannel] = field(default_factory=list)


@dataclass(frozen=True)
class InvestigationStep:
    id: str
    kind: str
    prompt: str
    answer: Optional[Answer] = None
    trace: list[TraceStep] = field(default_factory=list)
    annotation: Optional[str] = None
    executed_at: Optional[str] = None


@dataclass(frozen=True)
class Investigation:
    id: str
    seed_insight_id: str
    seed_source: str
    seed_dataset: str
    seed_title: str
    title: str
    owner: str
    steps: list[InvestigationStep] = field(default_factory=list)
    notes: list[dict[str, Any]] = field(default_factory=list)
    created_at: str = ""
    updated_at: str = ""


@dataclass(frozen=True)
class User:
    id: str
    email: str
    created_at: str


@dataclass(frozen=True)
class TokenResponse:
    token: Optional[str]
    expires_at: str


@dataclass(frozen=True)
class Session:
    session_token: str
    user: User


@dataclass(frozen=True)
class Finding:
    kind: str
    source: str
    dataset: str
    title: str
    summary: str
    severity: str
    confidence: float
    evidence: list[EvidenceRef] = field(default_factory=list)


@dataclass(frozen=True)
class PlayerEntry:
    """One named private-sector operator in a market-player group."""
    name: str
    note: str
    url: Optional[str] = None


@dataclass(frozen=True)
class MarketPlayerGroup:
    """A department's curated directory of related market players."""
    dept: str
    category: str
    players: list[PlayerEntry] = field(default_factory=list)
