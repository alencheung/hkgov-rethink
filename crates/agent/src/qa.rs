//! Natural-language Q&A (v6).
//!
//! [`run_agent_loop`] is the rich path (LLM-driven, multi-step). When no LLM is
//! configured — the default, no-key baseline — [`heuristic_answer`] provides a
//! deterministic fallback: it matches keywords in the question against ingested
//! dataset titles/names and summarizes what's in the store. This keeps the
//! `/v1/ask` endpoint useful end to end without external dependencies.
//!
//! Both paths return the same [`Answer`] shape (from [`crate::loop_mod`]), so
//! the API handler doesn't branch on provider.

use crate::loop_mod::Answer;
use crate::tools::ToolBelt;
use hkgov_common::Result;
use serde_json::{json, Value};

/// D-025: English stop-words that inflate dataset scores without carrying
/// meaning. Stripped from the question before token-overlap scoring so a
/// phrase like "what is the interbank liquidity?" scores on `interbank` +
/// `liquidity` only, not on `the` (which matches dozens of titles). The list
/// is deliberately small and deterministic — it is NOT a stemmer.
const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "are", "but", "not", "you", "all", "any", "can", "had", "her", "was",
    "one", "our", "out", "has", "have", "from", "this", "that", "with", "your", "what", "when",
    "where", "which", "there", "their", "about", "into", "than", "then", "them", "these", "those",
    "tell", "show", "give", "want", "know", "does", "did", "how", "who", "why", "its", "been",
    "were", "more", "some", "such", "only", "very", "also", "may", "might", "must", "should",
    "would", "could", "shall",
];

/// True when `word` is a stop-word (exact, lowercase match). Called only on
/// already-lowercased tokens, so no case-folding here.
fn is_stop_word(word: &str) -> bool {
    STOP_WORDS.contains(&word)
}

/// Produce a deterministic answer without an LLM. The strategy:
/// 1. Pull the dataset list from the store.
/// 2. Score each dataset by how many question tokens appear in its
///    title/name/source (case-insensitive substring match).
/// 3. If a top match stands out, summarize its record count + a sample of
///    fields. Otherwise, give a generic "here's what I have" answer.
///
/// D-025 fix: common English stop-words ("what", "is", "the", "of", …) and
/// very short tokens (≤2 chars) are stripped before scoring. With ~190
/// datasets in the catalog, stop-words matched many unrelated titles and
/// tied the real match, so the wrong dataset won on list-order tie-break.
/// Stripping them means only content tokens contribute to the score.
pub async fn heuristic_answer(question: &str, belt: &ToolBelt) -> Result<Answer> {
    let list = belt.invoke("list_datasets", &json!({})).await?;
    let datasets = list["datasets"].as_array().cloned().unwrap_or_default();

    let q_lower = question.to_ascii_lowercase();
    // D-025: drop stop-words + tiny tokens so they can't inflate unrelated
    // datasets' scores and win on a tie-break.
    let q_tokens: Vec<&str> = q_lower
        .split_whitespace()
        .filter(|t| t.len() > 2 && !is_stop_word(t))
        .collect();

    // Score each dataset by token overlap with its title + name + source + tags.
    // (Tags are included so a domain term like "hibor" — present only in tags,
    // not in the title — still matches the right dataset.)
    let mut scored: Vec<(usize, Value)> = datasets
        .iter()
        .map(|d| {
            let tags = d["tags"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            let haystack = format!(
                "{} {} {} {}",
                d["source"].as_str().unwrap_or(""),
                d["dataset"].as_str().unwrap_or(""),
                d["title"].as_str().unwrap_or(""),
                tags
            )
            .to_ascii_lowercase();
            let score = q_tokens
                .iter()
                .filter(|t| !t.is_empty() && haystack.contains(*t))
                .count();
            (score, d.clone())
        })
        .collect();
    scored.sort_by_key(|b| std::cmp::Reverse(b.0));

    if let Some((score, best)) = scored.first() {
        if *score > 0 {
            return summarize_match(best, belt).await;
        }
    }

    // No keyword match → generic inventory answer.
    let names: Vec<String> = datasets
        .iter()
        .map(|d| {
            format!(
                "{}/{}",
                d["source"].as_str().unwrap_or(""),
                d["dataset"].as_str().unwrap_or("")
            )
        })
        .collect();
    let text = if names.is_empty() {
        "I don't have any datasets ingested yet. Start the API and wait for the \
         ingest supervisor to warm the cache, then ask again."
            .to_string()
    } else {
        format!(
            "I couldn't match your question to a specific dataset. I currently \
             have these datasets: {}.",
            names.join(", ")
        )
    };
    Ok(Answer {
        text,
        confidence: 0.3,
        trace: vec![],
    })
}

/// Summarize one matched dataset: record count + a field sample.
async fn summarize_match(dataset: &Value, belt: &ToolBelt) -> Result<Answer> {
    let source = dataset["source"].as_str().unwrap_or("");
    let dataset_name = dataset["dataset"].as_str().unwrap_or("");
    let title = dataset["title"].as_str().unwrap_or(dataset_name);
    let count = dataset["record_count"].as_u64().unwrap_or(0);

    // Pull a small sample to name the available fields.
    let sample_args = json!({
        "source": source,
        "dataset": dataset_name,
        "limit": 1
    });
    let sample = belt
        .invoke("query_dataset", &sample_args)
        .await
        .unwrap_or_else(|_| json!({}));
    let fields: Vec<String> = sample["records"]
        .as_array()
        .and_then(|r| r.first())
        .map(|rec| {
            rec.as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let fields_str = if fields.is_empty() {
        "no fields".to_string()
    } else {
        fields.join(", ")
    };

    let text = format!(
        "{title} ({source}/{dataset_name}) has {count} record(s). \
         Fields: {fields_str}. (Answered in heuristic mode — set an LLM base URL \
         for deeper analysis.)"
    );
    Ok(Answer {
        text,
        confidence: 0.5,
        trace: vec![crate::loop_mod::TraceStep {
            tool: "query_dataset".into(),
            arguments: sample_args,
            result: sample,
        }],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hkgov_common::{DataSource, NormalizedRecord, RecordValue};
    use hkgov_store::{DatasetId, MemoryStore, RecordStore};
    use std::collections::BTreeMap;
    use std::sync::Arc;

    fn make_record(id: &str, field: &str, val: f64) -> NormalizedRecord {
        let mut fields = BTreeMap::new();
        fields.insert(field.into(), RecordValue::Float(val));
        NormalizedRecord {
            source: DataSource::Hkma,
            dataset: "daily-interbank-liquidity".into(),
            record_id: id.into(),
            fields,
            fetched_at: chrono::Utc::now(),
        }
    }

    async fn seeded_store() -> Arc<MemoryStore> {
        let store = Arc::new(MemoryStore::new(10, 60));
        let id = DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        store
            .register(
                id.clone(),
                "Daily Interbank Liquidity Figures".into(),
                None,
                3600,
                hkgov_common::Category::Monetary,
                vec!["hibor".into()],
                hkgov_common::Cadence::Daily,
            )
            .await;
        store
            .put_dataset(&id, vec![make_record("2026-01", "hibor_overnight", 2.0)])
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn heuristic_matches_known_keyword() {
        let store = seeded_store().await;
        let belt = ToolBelt::for_store(store);
        let ans = heuristic_answer("what is the interbank liquidity?", &belt)
            .await
            .unwrap();
        assert!(ans.text.contains("Daily Interbank Liquidity"));
        assert!(ans.confidence > 0.3);
        assert_eq!(ans.trace.len(), 1);
    }

    #[tokio::test]
    async fn heuristic_falls_back_to_inventory() {
        let store = seeded_store().await;
        let belt = ToolBelt::for_store(store);
        let ans = heuristic_answer("tell me about marigolds", &belt)
            .await
            .unwrap();
        // No keyword match → inventory fallback mentions the dataset.
        assert!(ans.text.contains("daily-interbank-liquidity"));
        assert!(ans.confidence <= 0.4);
    }

    #[tokio::test]
    async fn heuristic_empty_store_is_honest() {
        let store = Arc::new(hkgov_store::MemoryStore::new(10, 60));
        let belt = ToolBelt::for_store(store);
        let ans = heuristic_answer("anything", &belt).await.unwrap();
        assert!(ans.text.contains("don't have any datasets"));
    }

    // ---- D-025: stop-words must not let the wrong dataset win on a tie ----
    //
    // With a multi-dataset catalog, a question like "what is the interbank
    // liquidity?" used to match the wrong dataset because the stop-word "the"
    // inflated an unrelated title's score and tied the real match, which then
    // lost on list-order tie-break. After the fix only content tokens
    // ("interbank", "liquidity") score, so the right dataset wins.
    async fn multi_dataset_store() -> Arc<MemoryStore> {
        let store = Arc::new(MemoryStore::new(10, 60));
        // A distractor whose title contains the stop-word "the" but neither
        // content token — it must NOT win.
        let distractor = DatasetId::new(DataSource::Hkma, "hotlines-auth-retailbanks-rep");
        store
            .register(
                distractor.clone(),
                "List of hotlines for the authentication of callers claiming to be bank reps"
                    .into(),
                None,
                3600,
                hkgov_common::Category::Monetary,
                vec![],
                hkgov_common::Cadence::Daily,
            )
            .await;
        store
            .put_dataset(&distractor, vec![make_record("2026-01", "hotline_1", 0.0)])
            .await
            .unwrap();
        // The real target.
        let target = DatasetId::new(DataSource::Hkma, "daily-interbank-liquidity");
        store
            .register(
                target.clone(),
                "Daily Interbank Liquidity Figures".into(),
                None,
                3600,
                hkgov_common::Category::Monetary,
                vec!["hibor".into()],
                hkgov_common::Cadence::Daily,
            )
            .await;
        store
            .put_dataset(
                &target,
                vec![make_record("2026-01", "hibor_overnight", 2.0)],
            )
            .await
            .unwrap();
        store
    }

    #[tokio::test]
    async fn d025_stopwords_do_not_let_distractor_win() {
        let store = multi_dataset_store().await;
        let belt = ToolBelt::for_store(store);
        let ans = heuristic_answer("what is the interbank liquidity?", &belt)
            .await
            .unwrap();
        // Must match the interbank dataset, NOT the hotlines distractor.
        assert!(
            ans.text.contains("Interbank Liquidity"),
            "should match interbank, got: {}",
            ans.text
        );
        assert!(!ans.text.contains("hotlines"));
    }

    #[test]
    fn d025_is_stop_word_filters_common_words() {
        assert!(is_stop_word("the"));
        assert!(is_stop_word("what"));
        assert!(!is_stop_word("interbank"));
        assert!(!is_stop_word("hibor"));
        // Tiny tokens are filtered by length in the caller, not here.
        assert!(!is_stop_word("is"));
    }
}
