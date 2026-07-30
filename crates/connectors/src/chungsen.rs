//! Chung Sen Property Group (中誠地產) connector — 筍盤推介 / 銀主獨家 listings.
//!
//! Source: `chungsen.com.hk/tc/mortgage_property.php?wid=<id>` (verified live;
//! reachable from any egress — no proxy needed).
//!
//! The site exposes two listing pools behind query params:
//! - `wid=91` → page titled 筍盤推介 (hot picks)
//! - `wid=88` → page titled 銀主/獨家 (bank-owned / exclusive)
//!
//! **Important quirk (verified live July 2026):** both `wid` values return the
//! SAME 151-row HTML table — the param changes only the page title, not the
//! data. The connector therefore fetches BOTH pages (so we observe the labels
//! the operator has applied) and dedupes by 物業編號. A listing that appears
//! under both wid values is emitted ONCE with `page_label = "筍盤推介; 銀主/獨家"`.
//!
//! The listing rows are server-rendered HTML:
//! ```html
//! <tr>
//!   <td class="cont-text">大角咀中匯街3號中和樓1樓9室。<br />物業編號 : 260612-01</td>
//!   <td>976</td>           <!-- 建築面積 (sqft, build area) -->
//!   <td>667</td>           <!-- 實用面積 (sqft, saleable area) -->
//!   <td>588</td>           <!-- 售價(萬) — price in HK$10,000 -->
//! </tr>
//! ```
//!
//! Field shapes (verified against the live file):
//! - 物業編號 is a stable id like `260612-01` (YYMMDD-NN), parsed out of the
//!   address cell. This is the `record_id`.
//! - Build / saleable area are integers (sqft) but may be empty.
//! - 售價(萬) is per-HK$10,000 — `588` means HK$5.88M. It may be multi-valued
//!   for HOS units (居屋) with both 自由 (free market) and 居二 (second-hand
//!   HOS) tracks, e.g. `自由 348<br />居二 268`. We keep these as a string
//!   when multi-valued; the consumer can split if it needs the numeric.

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use std::sync::OnceLock;
use std::time::Duration;

/// The single dataset this connector exposes. Per the design (and the live
/// wid=88 ≡ wid=91 finding), one combined pool of listings.
const DATASET_ID: &str = "chungsen-listings";

static DATASETS: OnceLock<Vec<DatasetSpec>> = OnceLock::new();

fn datasets() -> &'static [DatasetSpec] {
    DATASETS.get_or_init(|| {
        vec![DatasetSpec {
            id: DATASET_ID,
            title: "Chung Sen Auction Listings (筍盤推介 + 銀主/獨家)",
            description: Some(
                "Chung Sen Property Group (中誠地產) public auction listings. \
                 Combined pool from the 筍盤推介 (hot picks, wid=91) and \
                 銀主/獨家 (bank-owned/exclusive, wid=88) pages — the site \
                 serves the same 151-row table under both labels, so each \
                 record carries the label(s) it appeared under. Fields: \
                 address (Chinese), build_area_sqft, saleable_area_sqft, \
                 price_10k (售價萬, may be multi-track for HOS units), \
                 page_label, source_url. record_id = 物業編號 (e.g. 260612-01).",
            ),
            category: Category::Property,
            tags: &[
                "chungsen",
                "auction",
                "bank-owned",
                "foreclosure",
                "筍盤推介",
                "銀主盤",
            ],
            cadence: Cadence::Daily,
            // Listings rotate on roughly weekly auction cycles; 6h keeps us
            // current without hammering a small site.
            refresh_interval_secs: 6 * 3600,
        }]
    })
}

const BASE_URL: &str = "https://www.chungsen.com.hk/tc/mortgage_property.php";

/// The two wid values we fetch. Despite both returning the same row set,
/// we fetch both so the operator-applied labels are observed and tagged
/// onto each record.
const WID_HOT_PICKS: u32 = 91; // 筍盤推介
const WID_BANK_EXCLUSIVE: u32 = 88; // 銀主/獨家

pub struct ChungSenConnector {
    client: reqwest::Client,
}

impl ChungSenConnector {
    pub fn new(_settings: &UpstreamSettings) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(30_000))
            .gzip(true)
            .redirect(reqwest::redirect::Policy::limited(5))
            .pool_max_idle_per_host(4)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;
        Ok(Self { client })
    }

    /// Fetch one wid page and return (page_label, parsed rows).
    async fn fetch_page(&self, wid: u32) -> Result<(String, Vec<ListingRow>)> {
        let url = format!("{BASE_URL}?wid={wid}");
        let resp = self
            .client
            .get(&url)
            .header("Accept-Language", "zh-HK,zh;q=0.9,en;q=0.8")
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "chungsen",
                status: 0,
                detail: format!("transport: {e}"),
            })?
            .error_for_status()
            .map_err(|e| Error::Upstream {
                origin: "chungsen",
                status: e.status().map(|s| s.as_u16()).unwrap_or(0),
                detail: format!("http: {e}"),
            })?;
        // Cap the listing HTML before parsing — a malformed/huge page would
        // otherwise OOM the process (PERF-CON-01).
        let body =
            crate::limited::read_text_limited(resp, "chungsen", crate::limited::MAX_DATA_BYTES)
                .await?;
        let page_label = extract_page_label(&body, wid);
        let rows = parse_listings(&body, wid, &page_label);
        Ok((page_label, rows))
    }
}

#[async_trait]
impl Connector for ChungSenConnector {
    fn source(&self) -> DataSource {
        DataSource::ChungSen
    }

    fn datasets(&self) -> &[DatasetSpec] {
        datasets()
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        if dataset != DATASET_ID {
            return Err(Error::Internal(format!(
                "chungsen: unknown dataset {dataset}"
            )));
        }
        // Fetch both pages in parallel. Both return the same row set, but the
        // labels differ — we merge by record_id below.
        let (hot, bank) = tokio::join!(
            self.fetch_page(WID_HOT_PICKS),
            self.fetch_page(WID_BANK_EXCLUSIVE),
        );
        let hot = hot?;
        let bank = bank?;

        tracing::info!(
            dataset = DATASET_ID,
            hot_picks_rows = hot.1.len(),
            bank_exclusive_rows = bank.1.len(),
            "chungsen: fetched both pages"
        );

        let now = Utc::now();
        let merged = merge_rows(&hot.1, &bank.1, now);
        Ok(merged)
    }
}

/// One parsed listing row from a chungsen page.
#[derive(Debug, Clone)]
struct ListingRow {
    /// 物業編號, e.g. `260612-01`. Stable across fetches.
    property_id: String,
    /// Address (Chinese), with the 物業編號 suffix stripped.
    address: String,
    build_area_sqft: Option<i64>,
    saleable_area_sqft: Option<i64>,
    /// Raw 售價(萬) cell. Kept as a string because it may be multi-valued
    /// (e.g. `自由 348<br />居二 268`); the consumer decides whether to
    /// parse out the numeric.
    price_10k_raw: String,
    /// Label of the page this row came from (筍盤推介 | 銀主/獨家).
    page_label: String,
    /// Full source URL this row was parsed from.
    source_url: String,
}

/// Pull the page heading (筍盤推介 / 銀主/獨家) out of the HTML so we can
/// tag each row with what the operator labeled it. Falls back to a wid-based
/// label if the heading isn't found.
fn extract_page_label(body: &str, wid: u32) -> String {
    // The heading appears inside the page as a standalone text node, e.g.
    // `<h1>筍盤推介</h1>` or as a section title. Look for either of the two
    // known labels anywhere in the body — the page only ever carries one.
    if body.contains("筍盤推介") {
        return "筍盤推介".into();
    }
    if body.contains("銀主/獨家") || body.contains("銀主／獨家") {
        return "銀主/獨家".into();
    }
    // Defensive fallback — should never trigger.
    match wid {
        WID_HOT_PICKS => "筍盤推介".into(),
        WID_BANK_EXCLUSIVE => "銀主/獨家".into(),
        _ => format!("wid={wid}"),
    }
}

/// Parse all listing rows out of one page body. Rows are `<tr>` blocks whose
/// first cell carries class `cont-text`.
fn parse_listings(body: &str, wid: u32, page_label: &str) -> Vec<ListingRow> {
    let source_url = format!("{BASE_URL}?wid={wid}");
    let mut rows = Vec::new();
    // Find each <tr> whose first cell is class="cont-text" — those are the
    // listing rows. The header <tr>s in <thead> don't have that class.
    for tr_match in find_listing_trs(body) {
        let cells = extract_td_cells(tr_match);
        if cells.is_empty() {
            continue;
        }
        // First cell: address + 物業編號 (HTML, may have <br> tags).
        let raw_first = &cells[0];
        let plain_first = strip_tags(raw_first);
        let (address, property_id) = split_address_and_id(&plain_first);
        if property_id.is_empty() {
            // Not a real listing row — skip.
            continue;
        }
        // Second cell: build area (建築面積). May be empty or multi-segment.
        let build_area_sqft = cells
            .get(1)
            .and_then(|c| parse_first_int(strip_tags(c).as_str()));
        // Third cell: saleable area (實用面積).
        let saleable_area_sqft = cells
            .get(2)
            .and_then(|c| parse_first_int(strip_tags(c).as_str()));
        // Fourth cell: price in 萬 (HK$10k). Keep raw — may be multi-valued.
        let price_10k_raw = cells
            .get(3)
            .map(|c| strip_tags(c).trim().to_string())
            .unwrap_or_default();
        rows.push(ListingRow {
            property_id,
            address,
            build_area_sqft,
            saleable_area_sqft,
            price_10k_raw,
            page_label: page_label.to_string(),
            source_url: source_url.clone(),
        });
    }
    rows
}

/// Merge rows from the two wid pages by `property_id`. A row seen under both
/// pages is emitted once with `page_label` joined by `; `.
fn merge_rows(
    hot: &[ListingRow],
    bank: &[ListingRow],
    now: chrono::DateTime<Utc>,
) -> Vec<NormalizedRecord> {
    use std::collections::BTreeMap;
    // key = property_id → (row, labels set)
    let mut by_id: BTreeMap<String, (ListingRow, Vec<String>)> = BTreeMap::new();
    for r in hot.iter().chain(bank.iter()) {
        let entry = by_id
            .entry(r.property_id.clone())
            .or_insert_with(|| (r.clone(), Vec::new()));
        let label = r.page_label.clone();
        if !entry.1.iter().any(|l| l == &label) {
            entry.1.push(label);
        }
    }
    by_id
        .into_iter()
        .map(|(id, (row, mut labels))| {
            labels.sort();
            labels.dedup();
            let mut fields = BTreeMap::new();
            fields.insert("address".into(), RecordValue::Str(row.address));
            if let Some(b) = row.build_area_sqft {
                fields.insert("build_area_sqft".into(), RecordValue::Int(b));
            }
            if let Some(s) = row.saleable_area_sqft {
                fields.insert("saleable_area_sqft".into(), RecordValue::Int(s));
            }
            if !row.price_10k_raw.is_empty() {
                // Stash the raw string and parse the numeric form before moving.
                let raw = row.price_10k_raw.clone();
                let parsed_int = raw.trim().parse::<i64>().ok();
                fields.insert("price_10k_raw".into(), RecordValue::Str(raw));
                if let Some(n) = parsed_int {
                    fields.insert("price_10k".into(), RecordValue::Int(n));
                }
            }
            fields.insert("page_label".into(), RecordValue::Str(labels.join("; ")));
            fields.insert("source_url".into(), RecordValue::Str(row.source_url));
            NormalizedRecord {
                source: DataSource::ChungSen,
                dataset: DATASET_ID.into(),
                record_id: id,
                fields,
                fetched_at: now,
            }
        })
        .collect()
}

// ---- tiny HTML helpers (the page is simple enough to avoid a full parser) ----

/// Find every `<tr>...</tr>` block whose first `<td>` has class `cont-text`.
/// Returns the slice of each matching tr (without the outer <tr> tags).
fn find_listing_trs(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(tr_start) = rest.find("<tr") {
        // Find the end of this <tr ...>.
        let after_open = if let Some(gt) = rest[tr_start..].find('>') {
            tr_start + gt + 1
        } else {
            break;
        };
        let close = match rest[after_open..].find("</tr>") {
            Some(c) => after_open + c,
            None => break,
        };
        let inner = &rest[after_open..close];
        // Check if the first <td ...> in this row has class="cont-text".
        if let Some(td_start) = inner.find("<td") {
            let td_tag_end = inner[td_start..]
                .find('>')
                .map(|p| td_start + p)
                .unwrap_or(td_start);
            let td_tag = &inner[td_start..=td_tag_end.min(inner.len().saturating_sub(1))];
            if td_tag.contains("cont-text") {
                out.push(inner);
            }
        }
        rest = &rest[close + 5..];
    }
    out
}

/// Extract the text content of each `<td>...</td>` cell in a row, preserving
/// inline HTML (caller will `strip_tags` as needed).
fn extract_td_cells(row_html: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut rest = row_html;
    while let Some(td_start) = rest.find("<td") {
        let after_open = match rest[td_start..].find('>') {
            Some(p) => td_start + p + 1,
            None => break,
        };
        let close = match rest[after_open..].find("</td>") {
            Some(c) => after_open + c,
            None => break,
        };
        cells.push(rest[after_open..close].to_string());
        rest = &rest[close + 5..];
    }
    cells
}

/// Strip HTML tags and collapse whitespace.
fn strip_tags(html: &str) -> String {
    // Replace <br> variants with newline first so multi-line cells survive.
    let with_newlines = html.replace("<br />", "\n").replace("<br>", "\n");
    let mut out = String::with_capacity(with_newlines.len());
    let mut in_tag = false;
    for ch in with_newlines.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }
    // Collapse runs of whitespace (but preserve newlines).
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_ws = false;
    for ch in out.chars() {
        let is_ws = ch == ' ' || ch == '\t';
        let is_nl = ch == '\n' || ch == '\r';
        if is_nl {
            collapsed.push(ch);
            prev_ws = false;
        } else if is_ws {
            if !prev_ws {
                collapsed.push(' ');
            }
            prev_ws = true;
        } else {
            collapsed.push(ch);
            prev_ws = false;
        }
    }
    collapsed.trim().to_string()
}

/// Split a "address 物業編號 : 260612-01" cell into (address, id).
/// The id is the YYMMDD-NN pattern after the 物業編號 marker.
fn split_address_and_id(plain: &str) -> (String, String) {
    // Find "物業編號" (property id marker). The colon may be ASCII or fullwidth.
    if let Some(idx) = plain.find("物業編號") {
        let address = plain[..idx]
            .trim()
            .trim_end_matches('。')
            .trim()
            .to_string();
        // Slice by BYTE length (find returns byte offset), not char count —
        // `idx + chars().count()` would land mid-codepoint on CJK text.
        let rest = &plain[idx + "物業編號".len()..];
        // Skip optional colon/space/全形空格 prefix.
        let rest = rest.trim_start_matches([' ', ':', '：', '\u{3000}']);
        // The id is the first run of [0-9-] characters.
        let id_end = rest
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_digit() || *c == '-')
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        let id = rest[..id_end].to_string();
        return (address, id);
    }
    (plain.trim().to_string(), String::new())
}

/// Parse the first integer run out of a string (allowing thousands separators).
fn parse_first_int(s: &str) -> Option<i64> {
    let cleaned: String = s
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit() || *c == ',' || *c == '.')
        .filter(|c| *c != ',')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    // Take the integer part if there's a decimal.
    let int_part = cleaned.split('.').next().unwrap_or(&cleaned);
    int_part.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_address_and_id() {
        let (addr, id) =
            split_address_and_id("大角咀中匯街3號中和樓1樓9室。\n物業編號 : 260612-01");
        assert_eq!(addr, "大角咀中匯街3號中和樓1樓9室");
        assert_eq!(id, "260612-01");

        // Fullwidth colon variant.
        let (addr, id) = split_address_and_id(
            "馬鞍山西沙路638號錦豐苑錦莉閣 (D座)31字樓1單位。\n物業編號: 260611-02",
        );
        assert_eq!(id, "260611-02");
        assert!(addr.contains("馬鞍山"));

        // No marker → address is the whole thing, no id.
        let (addr, id) = split_address_and_id("just an address with no id");
        assert_eq!(addr, "just an address with no id");
        assert_eq!(id, "");
    }

    #[test]
    fn parses_first_int_with_commas_and_decimals() {
        assert_eq!(parse_first_int("1,186"), Some(1186));
        assert_eq!(parse_first_int("656.7"), Some(656));
        assert_eq!(parse_first_int("建築(約) 796"), Some(796));
        assert_eq!(parse_first_int("no number here"), None);
        assert_eq!(parse_first_int(""), None);
    }

    #[test]
    fn strips_tags_and_preserves_newlines() {
        let s = strip_tags("元朗德業街11號映御2座3字樓E室。<br />\n物業編號 : 260612-06");
        assert!(s.contains("映御"));
        assert!(s.contains("物業編號"));
        assert!(s.contains('\n'));
    }

    #[test]
    fn parses_real_chungsen_sample_into_rows() {
        // Miniature version of the live table — one header row + three
        // listing rows (one with multi-track HOS pricing).
        let html = r#"<div class="Property-table-wrapper-rows">
            <table class="table table-bordered">
                <thead><tr>
                    <th>物業名稱</th><th>建築面積(約)<br>(平方呎)</th>
                    <th>實用面積 (約)<br>(平方呎)</th><th>售價(萬)</th>
                </tr></thead>
                <tbody>
                    <tr>
                        <td class="cont-text">大角咀中匯街3號中和樓1樓9室。<br />物業編號 : 260612-01</td>
                        <td></td><td>427</td><td></td>
                    </tr>
                    <tr>
                        <td class="cont-text">將軍澳寶琳北路20號英明苑E座19樓5室。<br />物業編號 : 260612-04</td>
                        <td>710</td><td>554</td><td>588</td>
                    </tr>
                    <tr>
                        <td class="cont-text">牛頭角振華道50號樂雅苑雅靜閣 (B座)3字樓7室。<br />物業編號: 260611-01</td>
                        <td></td><td>431</td><td>自由 348<br />居二 268</td>
                    </tr>
                </tbody>
            </table>
        </div>"#;
        let rows = parse_listings(html, WID_HOT_PICKS, "筍盤推介");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].property_id, "260612-01");
        assert_eq!(rows[0].build_area_sqft, None);
        assert_eq!(rows[0].saleable_area_sqft, Some(427));
        assert_eq!(rows[0].price_10k_raw, "");
        assert_eq!(rows[1].property_id, "260612-04");
        assert_eq!(rows[1].build_area_sqft, Some(710));
        assert_eq!(rows[1].saleable_area_sqft, Some(554));
        assert_eq!(rows[1].price_10k_raw, "588");
        // Multi-track HOS pricing is preserved as a string.
        assert!(rows[2].price_10k_raw.contains("自由"));
        assert!(rows[2].price_10k_raw.contains("居二"));
    }

    #[test]
    fn merge_dedupes_by_property_id_and_joins_labels() {
        let now = Utc::now();
        // Row appears in BOTH pages (same id) + a row only in hot picks.
        let hot = vec![
            ListingRow {
                property_id: "260612-01".into(),
                address: "Address A".into(),
                build_area_sqft: None,
                saleable_area_sqft: Some(427),
                price_10k_raw: "".into(),
                page_label: "筍盤推介".into(),
                source_url: "wid=91".into(),
            },
            ListingRow {
                property_id: "260612-04".into(),
                address: "Address B".into(),
                build_area_sqft: Some(710),
                saleable_area_sqft: Some(554),
                price_10k_raw: "588".into(),
                page_label: "筍盤推介".into(),
                source_url: "wid=91".into(),
            },
        ];
        let bank = vec![ListingRow {
            property_id: "260612-01".into(),
            address: "Address A".into(),
            build_area_sqft: None,
            saleable_area_sqft: Some(427),
            price_10k_raw: "".into(),
            page_label: "銀主/獨家".into(),
            source_url: "wid=88".into(),
        }];
        let merged = merge_rows(&hot, &bank, now);
        assert_eq!(merged.len(), 2, "two distinct property_ids");
        // The duplicate row should carry both labels, joined.
        let dup = merged.iter().find(|r| r.record_id == "260612-01").unwrap();
        let label = dup.fields.get("page_label").and_then(|v| match v {
            RecordValue::Str(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(label, Some("筍盤推介; 銀主/獨家"));
        // The hot-only row keeps just its label.
        let hot_only = merged.iter().find(|r| r.record_id == "260612-04").unwrap();
        let label = hot_only.fields.get("page_label").and_then(|v| match v {
            RecordValue::Str(s) => Some(s.as_str()),
            _ => None,
        });
        assert_eq!(label, Some("筍盤推介"));
        // Numeric price is exposed for the single-int case.
        assert_eq!(
            hot_only.fields.get("price_10k"),
            Some(&RecordValue::Int(588))
        );
    }
}
