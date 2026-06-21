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
