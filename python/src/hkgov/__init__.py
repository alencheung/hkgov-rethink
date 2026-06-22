"""Typed Python client for hkgov-rethink.

A thin, dependency-light wrapper over the HTTP API. Everything the API can do,
this client can do. See README.md for usage and the full method table.
"""

from __future__ import annotations

from .client import HkGov, HkGovError
from .models import (
    AlertLogEntry,
    Answer,
    Brief,
    BriefItem,
    CategoryGroup,
    DatasetMeta,
    Insight,
    EvidenceRef,
    Health,
    Record,
    RecordPage,
    SourceHealth,
    TraceStep,
)

__all__ = [
    "HkGov",
    "HkGovError",
    "Answer",
    "AlertLogEntry",
    "Brief",
    "BriefItem",
    "CategoryGroup",
    "DatasetMeta",
    "EvidenceRef",
    "Health",
    "Insight",
    "Record",
    "RecordPage",
    "SourceHealth",
    "TraceStep",
]
__version__ = "0.2.0"
