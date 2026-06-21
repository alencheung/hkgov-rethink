//! Agent tool belt — the deterministic substrate the agent layer calls.
//!
//! Each [`Tool`] wraps a read-only store query or a detector behind a uniform
//! `async fn call(args) -> Result<Value>` interface with an OpenAI-compatible
//! JSON schema. This is what lets the LLM-driven agent loop (Phase 3) and the
//! NL Q&A endpoint (Phase 4) invoke store reads and detection without either
//! side knowing the store's concrete shape.
//!
//! **Determinism guarantee:** every tool is read-only and pure with respect to
//! the store snapshot it reads. Detection never happens in the LLM — the LLM
//! only *selects* which tool to call and *frames* the result. This is the
//! "keep deterministic core" principle made concrete.
//!
//! The concrete tools:
//! - [`ListDatasetsTool`] — wraps `RecordStore::list`.
//! - [`QueryDatasetTool`] — wraps `RecordStore::get_page`, with optional
//!   field filtering so the LLM doesn't pull whole rows it doesn't need.
//! - [`RunDetectorTool`] — wraps any `analysis::*` detector by name.

use crate::analysis::{
    detect_correlation, detect_cross_source_gaps, detect_outliers, detect_seasonality,
    detect_series_jumps, Finding, DEFAULT_CORRELATION_R, DEFAULT_OUTLIER_Z, DEFAULT_SEASONALITY_R,
};
use async_trait::async_trait;
use hkgov_common::{DataSource, NormalizedRecord, RecordValue, Result};
use hkgov_store::{DatasetId, MemoryStore, RecordStore};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;

/// What every agent tool must provide. Implementations are read-only with
/// respect to the store — they never mutate ingested data.
#[async_trait]
pub trait Tool: Send + Sync + 'static {
    /// Stable, short identifier the LLM uses to call the tool.
    fn name(&self) -> &'static str;

    /// One-line description of what the tool does.
    fn description(&self) -> &'static str;

    /// OpenAI function-calling JSON schema for the tool's arguments. The shape
    /// is `{"type": "object", "properties": {...}, "required": [...]}`.
    fn schema(&self) -> Value;

    /// Execute the tool against the given (already-validated) JSON args.
    async fn call(&self, args: &Value) -> Result<Value>;
}

/// A collection of tools the agent loop can dispatch against. Tools are looked
/// up by name; unknown names return an error rather than panicking.
pub struct ToolBelt {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolBelt {
    pub fn new(tools: Vec<Box<dyn Tool>>) -> Self {
        Self { tools }
    }

    /// The default belt for an in-process store: list + query + run_detector.
    pub fn for_store(store: Arc<MemoryStore>) -> Self {
        Self::new(vec![
            Box::new(ListDatasetsTool::new(store.clone())),
            Box::new(QueryDatasetTool::new(store.clone())),
            Box::new(RunDetectorTool::new(store)),
        ])
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools
            .iter()
            .find(|t| t.name() == name)
            .map(|t| t.as_ref())
    }

    /// Schemas for every tool, in the shape OpenAI's `tools` field expects:
    /// a list of `{"type": "function", "function": {...}}` objects.
    pub fn tool_specs(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.schema(),
                    }
                })
            })
            .collect()
    }

    /// Dispatch a named tool call. Returns an error for unknown tools so the
    /// agent loop can surface the failure rather than silently no-op.
    pub async fn invoke(&self, name: &str, args: &Value) -> Result<Value> {
        match self.get(name) {
            Some(tool) => tool.call(args).await,
            None => Err(hkgov_common::Error::Internal(format!(
                "unknown tool: {name}"
            ))),
        }
    }
}

// ---------------------------------------------------------------------------
// ListDatasetsTool
// ---------------------------------------------------------------------------

/// List the datasets currently held in the store, optionally filtered by source.
pub struct ListDatasetsTool {
    store: Arc<MemoryStore>,
}

impl ListDatasetsTool {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for ListDatasetsTool {
    fn name(&self) -> &'static str {
        "list_datasets"
    }
    fn description(&self) -> &'static str {
        "List the datasets currently ingested in the store, with record counts \
         and last-refresh time. Optional `source` filters to one source \
         (hkma | datagovhk | press | landsd)."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "enum": ["hkma", "datagovhk", "press", "landsd"],
                    "description": "Optional source filter."
                }
            },
            "required": []
        })
    }
    async fn call(&self, args: &Value) -> Result<Value> {
        let source = args
            .get("source")
            .and_then(Value::as_str)
            .and_then(DataSource::parse);
        let metas = self.store.list(source).await?;
        let out: Vec<Value> = metas
            .iter()
            .map(|m| {
                json!({
                    "source": m.source.as_str(),
                    "dataset": m.dataset,
                    "title": m.title,
                    "record_count": m.record_count,
                    "last_refreshed_at": m.last_refreshed_at,
                })
            })
            .collect();
        Ok(json!({ "datasets": out }))
    }
}

// ---------------------------------------------------------------------------
// QueryDatasetTool
// ---------------------------------------------------------------------------

/// Read a page of records from one dataset. `fields` optionally projects the
/// result down to named fields (keeps payloads small when the LLM only needs
/// a couple of columns).
pub struct QueryDatasetTool {
    store: Arc<MemoryStore>,
}

impl QueryDatasetTool {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for QueryDatasetTool {
    fn name(&self) -> &'static str {
        "query_dataset"
    }
    fn description(&self) -> &'static str {
        "Read a page of records from one dataset. Pass `source`, `dataset`, and \
         optionally `offset` (default 0), `limit` (default 50, max 500), and \
         `fields` (a list of field names to project — omit for all fields)."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "source": {
                    "type": "string",
                    "enum": ["hkma", "datagovhk", "press", "landsd"]
                },
                "dataset": { "type": "string" },
                "offset": { "type": "integer", "minimum": 0, "default": 0 },
                "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 50 },
                "fields": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional projection — only these field names are returned."
                }
            },
            "required": ["source", "dataset"]
        })
    }
    async fn call(&self, args: &Value) -> Result<Value> {
        let source = args
            .get("source")
            .and_then(Value::as_str)
            .and_then(DataSource::parse)
            .ok_or_else(|| {
                hkgov_common::Error::UnknownSource(
                    args.get("source")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                )
            })?;
        let dataset = args
            .get("dataset")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                hkgov_common::Error::Internal("query_dataset: missing `dataset`".into())
            })?
            .to_string();
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(50)
            .clamp(1, 500) as usize;
        let fields: Option<Vec<String>> = args.get("fields").and_then(Value::as_array).map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        });

        let id = DatasetId::new(source, dataset.clone());
        let page = self.store.get_page(&id, offset, limit).await?;

        let records: Vec<Value> = page
            .records
            .iter()
            .map(|r| project_record(r, fields.as_deref()))
            .collect();

        Ok(json!({
            "source": page.source.as_str(),
            "dataset": page.dataset,
            "total": page.total,
            "offset": page.offset,
            "limit": page.limit,
            "records": records,
        }))
    }
}

/// Project a normalized record down to a subset of fields (or return all).
fn project_record(r: &NormalizedRecord, fields: Option<&[String]>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("record_id".into(), json!(r.record_id));
    match fields {
        Some(names) => {
            for n in names {
                obj.insert(n.clone(), record_value_json(r.fields.get(n)));
            }
        }
        None => {
            for (k, v) in &r.fields {
                obj.insert(k.clone(), record_value_json(Some(v)));
            }
        }
    }
    Value::Object(obj)
}

fn record_value_json(v: Option<&RecordValue>) -> Value {
    match v {
        Some(RecordValue::Null) | None => Value::Null,
        Some(RecordValue::Bool(b)) => json!(b),
        Some(RecordValue::Int(i)) => json!(i),
        Some(RecordValue::Float(f)) => json!(f),
        Some(RecordValue::Str(s)) => json!(s),
    }
}

// ---------------------------------------------------------------------------
// RunDetectorTool
// ---------------------------------------------------------------------------

/// Run one detector by name against one dataset's records. This is the bridge
/// from the tool belt to the deterministic detectors in `analysis.rs`.
///
/// Supported detectors (mirrors `[[agent.scan]]`):
/// `series_jump`, `outlier`, `seasonality`, `correlation`, `cross_source_gap`.
pub struct RunDetectorTool {
    store: Arc<MemoryStore>,
}

impl RunDetectorTool {
    pub fn new(store: Arc<MemoryStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl Tool for RunDetectorTool {
    fn name(&self) -> &'static str {
        "run_detector"
    }
    fn description(&self) -> &'static str {
        "Run one anomaly detector against a dataset and return the structured \
         findings it surfaces. Detectors: series_jump, outlier, seasonality, \
         correlation, cross_source_gap. Each returns findings with evidence \
         pointers back into the store."
    }
    fn schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "detector": {
                    "type": "string",
                    "enum": ["series_jump", "outlier", "seasonality", "correlation", "cross_source_gap"]
                },
                "source": { "type": "string", "enum": ["hkma", "datagovhk", "press", "landsd"] },
                "dataset": { "type": "string" },
                "field": {
                    "type": "string",
                    "description": "Numeric field for series_jump/outlier/seasonality, or the date column for cross_source_gap."
                },
                "field_b": {
                    "type": "string",
                    "description": "Second field, required for correlation."
                },
                "threshold": {
                    "type": "number",
                    "description": "Detector-specific threshold; omit for the documented default."
                },
                "companion_source": {
                    "type": "string",
                    "enum": ["hkma", "datagovhk", "press", "landsd"],
                    "description": "For cross_source_gap: the data-side source."
                },
                "companion_dataset": {
                    "type": "string",
                    "description": "For cross_source_gap: the data-side dataset whose record_ids are compared against `field`."
                }
            },
            "required": ["detector", "source", "dataset"]
        })
    }
    async fn call(&self, args: &Value) -> Result<Value> {
        let detector = args
            .get("detector")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                hkgov_common::Error::Internal("run_detector: missing `detector`".into())
            })?
            .to_string();
        let source = parse_source_arg(args)?;
        let dataset = args
            .get("dataset")
            .and_then(Value::as_str)
            .ok_or_else(|| hkgov_common::Error::Internal("run_detector: missing `dataset`".into()))?
            .to_string();
        let field = args.get("field").and_then(Value::as_str).map(String::from);
        let field_b = args
            .get("field_b")
            .and_then(Value::as_str)
            .map(String::from);
        let threshold = args.get("threshold").and_then(Value::as_f64);
        let companion_source = args.get("companion_source").and_then(Value::as_str);
        let companion_dataset = args.get("companion_dataset").and_then(Value::as_str);

        // cross_source_gap needs two datasets; handle separately.
        if detector == "cross_source_gap" {
            return run_gap_via_tool(
                &self.store,
                source,
                &dataset,
                field.as_deref(),
                companion_source,
                companion_dataset,
            )
            .await;
        }

        let id = DatasetId::new(source, dataset.clone());
        let page = self.store.get_page(&id, 0, 500).await?;
        let field = field.as_deref().ok_or_else(|| {
            hkgov_common::Error::Internal(format!("run_detector/{detector}: `field` required"))
        })?;

        let findings: Vec<Finding> = match detector.as_str() {
            "series_jump" => detect_series_jumps(
                source,
                &dataset,
                &page.records,
                field,
                threshold.unwrap_or(25.0),
            ),
            "outlier" => detect_outliers(
                source,
                &dataset,
                &page.records,
                field,
                threshold.unwrap_or(DEFAULT_OUTLIER_Z),
            ),
            "seasonality" => detect_seasonality(
                source,
                &dataset,
                &page.records,
                field,
                threshold.unwrap_or(DEFAULT_SEASONALITY_R),
            ),
            "correlation" => {
                let field_b = field_b.as_deref().ok_or_else(|| {
                    hkgov_common::Error::Internal(
                        "run_detector/correlation: `field_b` required".into(),
                    )
                })?;
                detect_correlation(
                    source,
                    &dataset,
                    &page.records,
                    field,
                    field_b,
                    threshold.unwrap_or(DEFAULT_CORRELATION_R),
                )
            }
            other => {
                return Err(hkgov_common::Error::Internal(format!(
                    "run_detector: unknown detector `{other}`"
                )))
            }
        };

        Ok(json!({ "detector": detector, "findings": findings }))
    }
}

fn parse_source_arg(args: &Value) -> Result<DataSource> {
    args.get("source")
        .and_then(Value::as_str)
        .and_then(DataSource::parse)
        .ok_or_else(|| {
            hkgov_common::Error::UnknownSource(
                args.get("source")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            )
        })
}

/// `cross_source_gap` via the tool interface: reads both sides from the store
/// and delegates to `detect_cross_source_gaps`.
async fn run_gap_via_tool(
    store: &Arc<MemoryStore>,
    source: DataSource,
    dataset: &str,
    date_field: Option<&str>,
    companion_source: Option<&str>,
    companion_dataset: Option<&str>,
) -> Result<Value> {
    let companion_source = companion_source
        .and_then(DataSource::parse)
        .ok_or_else(|| {
            hkgov_common::Error::Internal(
                "run_detector/cross_source_gap: `companion_source` required".into(),
            )
        })?;
    let companion_dataset = companion_dataset.ok_or_else(|| {
        hkgov_common::Error::Internal(
            "run_detector/cross_source_gap: `companion_dataset` required".into(),
        )
    })?;
    let date_field = date_field.unwrap_or("date");

    let press_id = DatasetId::new(source, dataset);
    let press_dates: Vec<String> = match store.get_page(&press_id, 0, 500).await {
        Ok(p) => p
            .records
            .iter()
            .filter_map(|r| match r.fields.get(date_field) {
                Some(RecordValue::Str(s)) => Some(s.clone()),
                _ => None,
            })
            .collect(),
        Err(e) => return Err(e),
    };
    let data_id = DatasetId::new(companion_source, companion_dataset);
    let data_dates: Vec<String> = match store.get_page(&data_id, 0, 500).await {
        Ok(p) => p.records.iter().map(|r| r.record_id.clone()).collect(),
        Err(e) => return Err(e),
    };

    let findings = detect_cross_source_gaps(source, dataset, &press_dates, &data_dates);
    Ok(json!({ "detector": "cross_source_gap", "findings": findings }))
}

/// Serialization shape a tool result uses for a single finding. Re-exported so
/// the agent loop and API handlers can build the same JSON the detectors emit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingDto {
    pub kind: String,
    pub source: String,
    pub dataset: String,
    pub title: String,
    pub heuristic_summary: String,
    pub severity: String,
    pub confidence: f64,
}

impl From<&Finding> for FindingDto {
    fn from(f: &Finding) -> Self {
        Self {
            kind: f.kind.clone(),
            source: f.source.to_string(),
            dataset: f.dataset.clone(),
            title: f.title.clone(),
            heuristic_summary: f.heuristic_summary.clone(),
            severity: f.severity.clone(),
            confidence: f.confidence,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hkgov_common::NormalizedRecord;
    use std::collections::BTreeMap;

    fn seed_record(
        store: &MemoryStore,
        source: DataSource,
        dataset: &str,
        records: Vec<NormalizedRecord>,
    ) {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let id = DatasetId::new(source, dataset);
            // `list()` reads from the registry, so we must register metadata
            // (mirroring what IngestSupervisor does) for list_datasets to see it.
            store
                .register(id.clone(), dataset.to_string(), None, 3600)
                .await;
            store.put_dataset(&id, records).await.unwrap();
        });
    }

    fn make_record(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut fields = BTreeMap::new();
        fields.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "x".into(),
            record_id: id.into(),
            fields,
            fetched_at: chrono::Utc::now(),
        }
    }

    fn rt() -> tokio::runtime::Runtime {
        tokio::runtime::Runtime::new().unwrap()
    }

    #[test]
    fn tool_specs_have_openai_shape() {
        let store = Arc::new(MemoryStore::new(10, 60));
        let belt = ToolBelt::for_store(store);
        let specs = belt.tool_specs();
        assert_eq!(specs.len(), 3);
        for s in &specs {
            assert_eq!(s["type"], "function");
            assert!(s["function"]["name"].is_string());
            assert!(s["function"]["description"].is_string());
            assert_eq!(s["function"]["parameters"]["type"], "object");
        }
        let names: Vec<&str> = specs
            .iter()
            .map(|s| s["function"]["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"list_datasets"));
        assert!(names.contains(&"query_dataset"));
        assert!(names.contains(&"run_detector"));
    }

    #[test]
    fn invoke_unknown_tool_errors() {
        let store = Arc::new(MemoryStore::new(10, 60));
        let belt = ToolBelt::for_store(store);
        let r = rt().block_on(belt.invoke("no_such_tool", &json!({})));
        assert!(r.is_err());
    }

    #[test]
    fn list_datasets_returns_seeded_data() {
        let store = Arc::new(MemoryStore::new(10, 60));
        seed_record(
            &store,
            DataSource::Hkma,
            "daily-interbank-liquidity",
            vec![make_record("2026-01", "v", 1.0)],
        );
        let belt = ToolBelt::for_store(store);
        let out = rt()
            .block_on(belt.invoke("list_datasets", &json!({})))
            .unwrap();
        let datasets = out["datasets"].as_array().unwrap();
        assert!(!datasets.is_empty());
        assert_eq!(datasets[0]["source"], "hkma");
    }

    #[test]
    fn query_dataset_projects_fields() {
        let store = Arc::new(MemoryStore::new(10, 60));
        let mut rec = make_record("2026-01", "a", 1.0);
        rec.fields.insert("b".into(), RecordValue::Float(2.0));
        seed_record(
            &store,
            DataSource::Hkma,
            "daily-interbank-liquidity",
            vec![rec],
        );
        let belt = ToolBelt::for_store(store);
        // Project only field "a" — "b" should be absent.
        let args = json!({
            "source": "hkma",
            "dataset": "daily-interbank-liquidity",
            "fields": ["a"]
        });
        let out = rt().block_on(belt.invoke("query_dataset", &args)).unwrap();
        let records = out["records"].as_array().unwrap();
        assert_eq!(records.len(), 1);
        assert!(records[0].get("a").is_some());
        assert!(records[0].get("b").is_none());
    }

    #[test]
    fn query_dataset_unknown_source_errors() {
        let store = Arc::new(MemoryStore::new(10, 60));
        let belt = ToolBelt::for_store(store);
        let args = json!({"source": "nonsense", "dataset": "x"});
        let r = rt().block_on(belt.invoke("query_dataset", &args));
        assert!(r.is_err());
    }

    #[test]
    fn run_detector_series_jump_finds_jump() {
        let store = Arc::new(MemoryStore::new(10, 60));
        seed_record(
            &store,
            DataSource::Hkma,
            "daily-interbank-liquidity",
            vec![
                make_record("2026-01", "hibor_overnight", 2.0),
                make_record("2026-02", "hibor_overnight", 6.0),
            ],
        );
        let belt = ToolBelt::for_store(store);
        let args = json!({
            "detector": "series_jump",
            "source": "hkma",
            "dataset": "daily-interbank-liquidity",
            "field": "hibor_overnight",
            "threshold": 50.0
        });
        let out = rt().block_on(belt.invoke("run_detector", &args)).unwrap();
        let findings = out["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["kind"], "series_jump");
    }

    #[test]
    fn run_detector_outlier_works() {
        let store = Arc::new(MemoryStore::new(10, 60));
        let baseline = [9.8_f64, 10.1, 9.9, 10.2, 10.0, 9.7];
        let mut recs: Vec<NormalizedRecord> = baseline
            .iter()
            .enumerate()
            .map(|(i, v)| make_record(&format!("2026-{i:02}"), "v", *v))
            .collect();
        recs.push(make_record("2026-99", "v", 100.0));
        seed_record(&store, DataSource::Hkma, "x", recs);
        let belt = ToolBelt::for_store(store);
        let args = json!({
            "detector": "outlier",
            "source": "hkma",
            "dataset": "x",
            "field": "v"
        });
        let out = rt().block_on(belt.invoke("run_detector", &args)).unwrap();
        assert!(!out["findings"].as_array().unwrap().is_empty());
    }

    #[test]
    fn run_detector_unknown_detector_errors() {
        let store = Arc::new(MemoryStore::new(10, 60));
        seed_record(
            &store,
            DataSource::Hkma,
            "x",
            vec![make_record("2026-01", "v", 1.0)],
        );
        let belt = ToolBelt::for_store(store);
        let args = json!({
            "detector": "bogus",
            "source": "hkma",
            "dataset": "x",
            "field": "v"
        });
        let r = rt().block_on(belt.invoke("run_detector", &args));
        assert!(r.is_err());
    }

    #[test]
    fn run_detector_cross_source_gap_via_tool() {
        let store = Arc::new(MemoryStore::new(10, 60));
        // Press side: two dates.
        let press = vec![
            NormalizedRecord {
                source: DataSource::Press,
                dataset: "hkma-press-releases".into(),
                record_id: "r1".into(),
                fields: {
                    let mut m = BTreeMap::new();
                    m.insert("date".into(), RecordValue::Str("2026-06-18".into()));
                    m
                },
                fetched_at: chrono::Utc::now(),
            },
            NormalizedRecord {
                source: DataSource::Press,
                dataset: "hkma-press-releases".into(),
                record_id: "r2".into(),
                fields: {
                    let mut m = BTreeMap::new();
                    m.insert("date".into(), RecordValue::Str("2026-06-19".into()));
                    m
                },
                fetched_at: chrono::Utc::now(),
            },
        ];
        seed_record(&store, DataSource::Press, "hkma-press-releases", press);
        // Data side: only one matching date.
        seed_record(
            &store,
            DataSource::Hkma,
            "daily-interbank-liquidity",
            vec![make_record("2026-06-18", "v", 1.0)],
        );

        let belt = ToolBelt::for_store(store);
        let args = json!({
            "detector": "cross_source_gap",
            "source": "press",
            "dataset": "hkma-press-releases",
            "field": "date",
            "companion_source": "hkma",
            "companion_dataset": "daily-interbank-liquidity"
        });
        let out = rt().block_on(belt.invoke("run_detector", &args)).unwrap();
        let findings = out["findings"].as_array().unwrap();
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0]["kind"], "cross_source_gap");
    }
}
