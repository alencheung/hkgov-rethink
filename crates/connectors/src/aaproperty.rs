//! AA Property Auctioneers (環亞物業拍賣) connector — public auction lot list.
//!
//! Source: `aaproperty.com.hk/aa/bid_list.php` (verified live; reachable
//! from any egress — no proxy needed). One row per auction lot, rendered as
//! legacy `<TR>` blocks (the site is old-school HTML 4).
//!
//! Each lot row has these cells (verified against the live page):
//!   0. (image cell — empty placeholder, lot photo on the detail page)
//!   1. lot number (`<b>` wrapped, e.g. `54`)
//!   2. address + 物業編號 (e.g. `新界元朗八鄉田心村543號… (物業編號: 000…)`)
//!   3. property type (`空地`, `住宅`, `工商`, etc.)
//!   4. occupancy (`交吉`, `不交吉`, `連租約`)
//!   5. area (interleaved 建築/地段/實用 cells, e.g.
//!      `建築(約) 地段(約) 實用(約) 372 (未核實)`)
//!   6. an integer (likely the photo count)
//!   7. price hint (`歡迎查詢` = on enquiry, or a dollar value)
//!   8. agent phone
//!
//! The page also carries an auction-date/venue banner near the top:
//!   `拍賣日期及地點 : 2026/07/07 下午3:00-5:00 | 九龍尖沙咀… | 2026/07/21 下…`
//! We capture each auction session as a dataset-level record keyed by date.

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use std::time::Duration;

const DATASET_ID: &str = "aaproperty-auction-list";
const AUCTION_SESSIONS_ID: &str = "aaproperty-auction-sessions";

static DATASETS: OnceLock<Vec<DatasetSpec>> = OnceLock::new();

fn datasets() -> &'static [DatasetSpec] {
    DATASETS.get_or_init(|| {
        vec![
            DatasetSpec {
                id: DATASET_ID,
                title: "AA Property Auction — Lot List (環亞物業拍賣)".into(),
                description: Some(
                    "AA Property Auctioneers (環亞物業拍賣) public auction lot \
                     list. One record per lot on the next scheduled auction. \
                     Fields: lot_no, address, property_type (空地/住宅/工商/…), \
                     occupancy (交吉/連租約/…), area_sqft, price_hint \
                     (often 歡迎查詢 = on enquiry), agent_phone, source_url. \
                     record_id = 物業編號. These are open-auction listings — \
                     distinct from the bank-owned foreclosure pools at Midland \
                     and Chung Sen."
                        .into(),
                ),
                category: Category::Property,
                tags: &["aaproperty", "auction", "環亞", "公開拍賣"],
                cadence: Cadence::Daily,
                refresh_interval_secs: 6 * 3600,
            },
            DatasetSpec {
                id: AUCTION_SESSIONS_ID,
                title: "AA Property Auction — Upcoming Sessions".into(),
                description: Some(
                    "AA Property upcoming auction sessions parsed from the \
                     bid-list page banner. One record per session, keyed by \
                     date. Fields: date, time, venue. Lets the agent layer \
                     join 'next auction date' to lot counts and address \
                     clusters."
                        .into(),
                ),
                category: Category::Property,
                tags: &["aaproperty", "auction", "session", "schedule"],
                cadence: Cadence::Daily,
                refresh_interval_secs: 6 * 3600,
            },
        ]
    })
}

const BID_LIST_URL: &str = "https://www.aaproperty.com.hk/aa/bid_list.php";

pub struct AaPropertyConnector {
    client: reqwest::Client,
}

impl AaPropertyConnector {
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

    async fn fetch_body(&self) -> Result<String> {
        self.client
            .get(BID_LIST_URL)
            .header("Accept-Language", "zh-HK,zh;q=0.9,en;q=0.8")
            .send()
            .await
            .map_err(|e| Error::Upstream {
                origin: "aaproperty",
                status: 0,
                detail: format!("transport: {e}"),
            })?
            .error_for_status()
            .map_err(|e| Error::Upstream {
                origin: "aaproperty",
                status: e.status().map(|s| s.as_u16()).unwrap_or(0),
                detail: format!("http: {e}"),
            })?
            .text()
            .await
            .map_err(|e| Error::Upstream {
                origin: "aaproperty",
                status: 0,
                detail: format!("body read: {e}"),
            })
    }
}

#[async_trait]
impl Connector for AaPropertyConnector {
    fn source(&self) -> DataSource {
        DataSource::AaProperty
    }

    fn datasets(&self) -> &[DatasetSpec] {
        datasets()
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let body = self.fetch_body().await?;
        let now = Utc::now();
        match dataset {
            DATASET_ID => {
                let rows = parse_lots(&body);
                tracing::info!(dataset, lots = rows.len(), "aaproperty: parsed lots");
                Ok(rows.into_iter().map(|r| r.into_record(now)).collect())
            }
            AUCTION_SESSIONS_ID => {
                let sessions = parse_sessions(&body, now);
                tracing::info!(
                    dataset,
                    sessions = sessions.len(),
                    "aaproperty: parsed upcoming sessions"
                );
                Ok(sessions)
            }
            other => Err(Error::Internal(format!(
                "aaproperty: unknown dataset {other}"
            ))),
        }
    }
}

/// One parsed lot row.
#[derive(Debug, Clone)]
struct LotRow {
    lot_no: String,
    property_id: String,
    address: String,
    property_type: String,
    occupancy: String,
    /// The first integer found in the area cell — best-effort sqft.
    area_sqft: Option<i64>,
    price_hint: String,
    agent_phone: String,
}

impl LotRow {
    fn into_record(self, now: DateTime<Utc>) -> NormalizedRecord {
        let mut fields = BTreeMap::new();
        // Decide record_id up front so we can move the strings freely below.
        // Use lot_no as the fallback record_id when the property_id is missing
        // (some lots — raw land, unusual items — don't carry a 物業編號).
        let record_id = if !self.property_id.is_empty() {
            self.property_id.clone()
        } else {
            format!("lot-{}", self.lot_no)
        };
        fields.insert("lot_no".into(), RecordValue::Str(self.lot_no));
        fields.insert("address".into(), RecordValue::Str(self.address));
        fields.insert("property_type".into(), RecordValue::Str(self.property_type));
        fields.insert("occupancy".into(), RecordValue::Str(self.occupancy));
        if let Some(a) = self.area_sqft {
            fields.insert("area_sqft".into(), RecordValue::Int(a));
        }
        if !self.price_hint.is_empty() {
            fields.insert("price_hint".into(), RecordValue::Str(self.price_hint));
        }
        if !self.agent_phone.is_empty() {
            fields.insert("agent_phone".into(), RecordValue::Str(self.agent_phone));
        }
        fields.insert(
            "source_url".into(),
            RecordValue::Str(BID_LIST_URL.to_string()),
        );
        NormalizedRecord {
            source: DataSource::AaProperty,
            dataset: DATASET_ID.into(),
            record_id,
            fields,
            fetched_at: now,
        }
    }
}

/// Parse all lot rows out of the bid-list page body. The lots live in
/// `<TR>...</TR>` blocks; each lot row contains a link to `item.php?item_no=…`
/// (the per-lot detail page) — that's our reliable marker.
fn parse_lots(body: &str) -> Vec<LotRow> {
    let mut lots = Vec::new();
    // Walk every <TR ...> ... </TR> block. (The site uses uppercase <TR>.)
    for tr in find_trs(body) {
        // The lot rows are the ones that link to item.php — the table skeleton
        // rows don't.
        if !tr.to_lowercase().contains("item.php") {
            continue;
        }
        let cells = extract_td_cells(tr);
        if cells.len() < 7 {
            continue;
        }
        // Find the lot-no cell (a <b>-wrapped integer).
        let mut lot_no = String::new();
        let mut lot_idx = None;
        for (i, c) in cells.iter().enumerate() {
            let t = strip_tags(c);
            if t.trim().chars().all(|ch| ch.is_ascii_digit()) && !t.trim().is_empty() {
                lot_no = t.trim().to_string();
                lot_idx = Some(i);
                break;
            }
        }
        let Some(lot_idx) = lot_idx else { continue };
        // The address cell is the one containing 物業編號.
        let mut address_cell_idx: Option<usize> = None;
        for (i, c) in cells.iter().enumerate() {
            if strip_tags(c).contains("物業編號") {
                address_cell_idx = Some(i);
                break;
            }
        }
        let Some(addr_idx) = address_cell_idx else {
            continue;
        };
        let (address, property_id) = split_address_and_id(&strip_tags(&cells[addr_idx]));
        // property_type, occupancy, area, price_hint, agent come from the
        // remaining cells after the address cell — the live layout has them
        // in a stable order, but we scan by content to be robust.
        let rest: Vec<String> = cells
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != lot_idx && *i != addr_idx)
            .map(|(_, c)| strip_tags(c))
            .collect();
        let property_type = rest
            .iter()
            .find(|c| is_property_type(c))
            .cloned()
            .unwrap_or_default();
        let occupancy = rest
            .iter()
            .find(|c| is_occupancy(c))
            .cloned()
            .unwrap_or_default();
        // Area cell: the one containing 建築/實用/地段 + a digit.
        let area_cell = rest
            .iter()
            .find(|c| {
                (c.contains("建築") || c.contains("實用") || c.contains("地段"))
                    && c.chars().any(|ch| ch.is_ascii_digit())
            })
            .cloned()
            .unwrap_or_default();
        let area_sqft = parse_first_int(&area_cell);
        // Price hint: 歡迎查詢 or a dollar value.
        let price_hint = rest
            .iter()
            .find(|c| {
                c.contains("歡迎查詢") || c.contains("詢價") || c.contains("萬") || c.contains("元")
            })
            .cloned()
            .unwrap_or_default();
        // Agent phone: a cell with phone-shaped digits.
        let agent_phone = rest
            .iter()
            .find(|c| {
                let digits: String = c.chars().filter(|ch| ch.is_ascii_digit()).collect();
                digits.len() >= 8
            })
            .cloned()
            .unwrap_or_default();
        lots.push(LotRow {
            lot_no,
            property_id,
            address,
            property_type,
            occupancy,
            area_sqft,
            price_hint,
            agent_phone,
        });
    }
    lots
}

/// Parse auction sessions out of the page banner. The live banner has a shape
/// like:
///   `拍賣日期及地點 : 2026/07/07 下午3:00-5:00 | 九龍尖沙咀… 2026/07/21 下午3:00-5:00 | 同上`
///
/// Sessions can be `|`-separated OR run together (venue + next date in one
/// text run). We scan the whole plain-text banner left-to-right, locating
/// each `YYYY/MM/DD` date prefix. For each session:
///   - the date is the prefix
///   - the time window runs from after the date until the next `YYYY/MM/DD`
///     (or end of banner)
///   - the venue is the trailing text in that time window AFTER stripping
///     the actual time-of-day part (下午3:00-5:00 etc.) — anything left after
///     the time-of-day is the venue.
///
/// This handles both the piped layout (clean per-segment venues) and the
/// run-on layout (venue + next date in one segment).
fn parse_sessions(body: &str, now: DateTime<Utc>) -> Vec<NormalizedRecord> {
    let marker = "拍賣日期及地點";
    let Some(marker_idx) = body.find(marker) else {
        return Vec::new();
    };
    let tail_end = (marker_idx + marker.len() + 2000).min(body.len());
    let tail = &body[marker_idx + marker.len()..tail_end];
    let plain = strip_tags(tail);
    // Find every date-prefix position in the plain text.
    let mut date_starts: Vec<usize> = Vec::new();
    let bytes = plain.as_bytes();
    let mut i = 0;
    while i + 6 <= bytes.len() {
        // YYYY/MM (4 digits, slash, 1-2 digits, slash) — anchor on a 4-digit
        // year that's NOT preceded by another digit (so we don't false-match
        // inside a longer number).
        let prev_ok = i == 0 || !bytes[i - 1].is_ascii_digit();
        if prev_ok
            && bytes[i..i + 4].iter().all(|b| b.is_ascii_digit())
            && bytes[i + 4] == b'/'
            && bytes[i + 5].is_ascii_digit()
        {
            date_starts.push(i);
            i += 6;
        } else {
            i += 1;
        }
    }
    // For each date, take the slice from its start to the next date's start
    // (or end of plain). Then split that slice into (date, time, venue).
    let mut sessions: Vec<(String, String, String)> = Vec::new();
    for (idx, &start) in date_starts.iter().enumerate() {
        let end = date_starts.get(idx + 1).copied().unwrap_or(plain.len());
        let chunk = plain[start..end].trim();
        if let Some((iso, time_window, venue)) = parse_session_datetime(chunk) {
            sessions.push((iso, time_window, venue));
        }
    }
    sessions
        .into_iter()
        .map(|(date_iso, time_window, venue)| {
            let mut fields = BTreeMap::new();
            fields.insert("date".into(), RecordValue::Str(date_iso.clone()));
            if !time_window.is_empty() {
                fields.insert("time".into(), RecordValue::Str(time_window));
            }
            if !venue.is_empty() {
                fields.insert("venue".into(), RecordValue::Str(venue));
            }
            NormalizedRecord {
                source: DataSource::AaProperty,
                dataset: AUCTION_SESSIONS_ID.into(),
                record_id: date_iso,
                fields,
                fetched_at: now,
            }
        })
        .collect()
}

/// Parse one date-led chunk (already sliced to span only this session) into
/// `(iso_date, time_window, venue)`. The chunk has shape:
///   `2026/07/07 下午3:00-5:00 九龍尖沙咀… venue text`
/// i.e. date, optional time-of-day, then trailing venue text.
fn parse_session_datetime(chunk: &str) -> Option<(String, String, String)> {
    let chunk = chunk.trim();
    let (year_str, rest) = split_leading_digits(chunk, |c: char| c == '/')?;
    let year: i32 = year_str.parse().ok()?;
    let rest = rest.trim_start_matches('/');
    let (month_str, rest) = split_leading_digits(rest, |c: char| c == '/')?;
    let month: u32 = month_str.parse().ok()?;
    let rest = rest.trim_start_matches('/');
    let (day_str, rest) = split_leading_digits(rest, |c: char| !c.is_ascii_digit())?;
    let day: u32 = day_str.parse().ok()?;
    let date = NaiveDate::from_ymd_opt(year, month, day)?;
    let iso = date.format("%Y-%m-%d").to_string();
    // `rest` is " 下午3:00-5:00 九龍尖沙咀… " or similar. Split time vs venue.
    // The time window may include the CJK markers 上午/下午 (AM/PM) before the
    // digits; the venue starts at the first CJK character that is NOT one of
    // those time markers. We walk char-by-char: any CJK char that follows a
    // digit (or another CJK that already followed a digit) is venue.
    let chars: Vec<(usize, char)> = rest.char_indices().collect();
    let mut venue_start: Option<usize> = None;
    let mut seen_digit = false;
    for &(i, c) in &chars {
        if c.is_ascii_digit() {
            seen_digit = true;
            continue;
        }
        // CJK after we've seen a digit → start of venue.
        let is_cjk = (c as u32) >= 0x4E00;
        if is_cjk && seen_digit {
            venue_start = Some(i);
            break;
        }
    }
    let (time_window, venue) = match venue_start {
        Some(i) => (rest[..i].trim().to_string(), rest[i..].trim().to_string()),
        None => (rest.trim().to_string(), String::new()),
    };
    Some((iso, time_window, venue))
}

/// Read leading digits up to (and excluding) `terminator`. Returns
/// `(digits, rest_after_terminator)`.
fn split_leading_digits<F: Fn(char) -> bool>(s: &str, is_term: F) -> Option<(String, &str)> {
    let mut digits = String::new();
    let mut rest = s;
    for (i, ch) in s.char_indices() {
        if ch.is_ascii_digit() {
            digits.push(ch);
        } else {
            rest = &s[i..];
            if !is_term(ch) {
                return None;
            }
            return Some((digits, rest));
        }
    }
    if digits.is_empty() {
        None
    } else {
        Some((digits, rest))
    }
}

// ---- HTML helpers (shared shape with chungsen.rs but kept local for clarity) ----

fn find_trs(body: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let lower = body.to_lowercase();
    let mut rest_idx = 0;
    while let Some(rel) = lower[rest_idx..].find("<tr") {
        let tr_start = rest_idx + rel;
        let after_open = match body[tr_start..].find('>') {
            Some(p) => tr_start + p + 1,
            None => break,
        };
        let close = match body[after_open..].to_lowercase().find("</tr>") {
            Some(c) => after_open + c,
            None => break,
        };
        out.push(&body[after_open..close]);
        rest_idx = close + 5;
    }
    out
}

fn extract_td_cells(row_html: &str) -> Vec<String> {
    let lower = row_html.to_lowercase();
    let mut cells = Vec::new();
    let mut rest_idx = 0;
    while let Some(rel) = lower[rest_idx..].find("<td") {
        let td_start = rest_idx + rel;
        let after_open = match row_html[td_start..].find('>') {
            Some(p) => td_start + p + 1,
            None => break,
        };
        let close_rel = match row_html[after_open..].to_lowercase().find("</td>") {
            Some(c) => c,
            None => break,
        };
        let close = after_open + close_rel;
        cells.push(row_html[after_open..close].to_string());
        rest_idx = close + 5;
    }
    cells
}

fn strip_tags(html: &str) -> String {
    let with_newlines = html
        .replace("<br />", "\n")
        .replace("<br>", "\n")
        .replace("<BR>", "\n");
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
    // Collapse non-newline whitespace.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_ws = false;
    for ch in out.chars() {
        if ch == '\n' || ch == '\r' {
            collapsed.push(ch);
            prev_ws = false;
        } else if ch == ' ' || ch == '\t' {
            if !prev_ws {
                collapsed.push(' ');
            }
            prev_ws = true;
        } else {
            collapsed.push(ch);
            prev_ws = false;
        }
    }
    // Decode the few entities the page actually uses.
    collapsed
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

fn split_address_and_id(plain: &str) -> (String, String) {
    if let Some(idx) = plain.find("物業編號") {
        let address = plain[..idx]
            .trim()
            .trim_end_matches('。')
            .trim()
            .to_string();
        // Slice by BYTE length (find returns byte offset) — char-count
        // arithmetic would land mid-codepoint on CJK text.
        let rest = &plain[idx + "物業編號".len()..];
        let rest = rest.trim_start_matches(|c: char| matches!(c, ' ' | ':' | '：' | '\u{3000}'));
        let id_end = rest
            .char_indices()
            .take_while(|(_, c)| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        let id = rest[..id_end].trim_end_matches(')').to_string();
        return (address, id);
    }
    (plain.trim().to_string(), String::new())
}

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
    cleaned.split('.').next().unwrap_or(&cleaned).parse().ok()
}

fn is_property_type(s: &str) -> bool {
    // Common property-type labels on AA Property. Conservatively: short
    // (≤8 chars), non-numeric, and matches one of the known type tokens.
    let trimmed = s.trim();
    if trimmed.len() > 12 || trimmed.is_empty() {
        return false;
    }
    if trimmed.chars().any(|c| c.is_ascii_digit()) {
        return false;
    }
    matches!(
        trimmed,
        "空地"
            | "住宅"
            | "工商"
            | "舖位"
            | "商業"
            | "工業"
            | "車位"
            | "獨立屋"
            | "村屋"
            | "寫字樓"
            | "工廈"
    )
}

fn is_occupancy(s: &str) -> bool {
    let trimmed = s.trim();
    matches!(
        trimmed,
        "交吉" | "不交吉" | "連租約" | "交吉連約" | "交吉 (租賃)" | "空置"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_address_and_id_with_parens() {
        let (addr, id) =
            split_address_and_id("新界元朗八鄉田心村543號金爵花園2期空地 (物業編號: 000123)");
        assert!(addr.starts_with("新界元朗"));
        assert_eq!(id, "000123");
    }

    #[test]
    fn parses_session_datetime_chinese_afternoon() {
        let (date, time, venue) = parse_session_datetime("2026/07/07 下午3:00-5:00").unwrap();
        assert_eq!(date, "2026-07-07");
        assert_eq!(time, "下午3:00-5:00");
        assert_eq!(venue, "");

        let (date, _, _) = parse_session_datetime("2026/7/7 上午10:00").unwrap();
        assert_eq!(date, "2026-07-07");

        // With a trailing venue.
        let (date, time, venue) =
            parse_session_datetime("2026/07/07 下午3:00-5:00 九龍尖沙咀彌敦道222號").unwrap();
        assert_eq!(date, "2026-07-07");
        assert_eq!(time, "下午3:00-5:00");
        assert!(venue.contains("九龍"));
    }

    #[test]
    fn parses_sessions_from_banner_sample() {
        let banner = r#"<div style="padding-left:20px;"> 拍賣日期及地點 :
            <span onclick="go()"> 2026/07/07 下午3:00-5:00 | 九龍尖沙咀彌敦道222號恆豐酒店2字樓宴會廳 2026/07/21 下午3:00-5:00 | 同上 </span>
        </div>"#;
        let now = Utc::now();
        let sessions = parse_sessions(banner, now);
        assert_eq!(sessions.len(), 2, "two sessions parsed");
        assert_eq!(sessions[0].record_id, "2026-07-07");
        let venue0 = sessions[0].fields.get("venue").and_then(|v| match v {
            RecordValue::Str(s) => Some(s.as_str()),
            _ => None,
        });
        assert!(venue0.unwrap_or("").contains("尖沙咀"));
        assert_eq!(sessions[1].record_id, "2026-07-21");
    }

    #[test]
    fn parses_lot_row_from_aaproperty_sample() {
        // Miniature version of one lot <TR> from the live page.
        let html = r#"<TR>
          <TD height=50 vAlign=center align=center>
            <a href="item.php?item_no=20260710"><IMG border=0 src="upload/realestate/20260710_01.jpg" width=50 height=40></a>
          </TD>
          <TD bgColor=white height=50 align=center>
            <FONT size=3 face=arial><b>54</b></FONT>
          </TD>
          <TD bgColor=white height=50>
            <FONT size=2 face=arial>新界元朗八鄉田心村543號金爵花園2期空地 (物業編號: 000123)</FONT>
          </TD>
          <TD>空地</TD>
          <TD>交吉</TD>
          <TD>建築(約) 地段(約) 實用(約) 372 (未核實)</TD>
          <TD>65</TD>
          <TD>歡迎查詢</TD>
          <TD>9190 0243 潘小姐 9222 5212 孫小姐</TD>
        </TR>"#;
        let rows = parse_lots(html);
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.lot_no, "54");
        assert_eq!(r.property_id, "000123");
        assert!(r.address.contains("元朗"));
        assert_eq!(r.property_type, "空地");
        assert_eq!(r.occupancy, "交吉");
        assert_eq!(r.area_sqft, Some(372));
        assert_eq!(r.price_hint, "歡迎查詢");
        assert!(r.agent_phone.contains("9190"));
    }

    #[test]
    fn recognizes_property_type_and_occupancy_labels() {
        assert!(is_property_type("空地"));
        assert!(is_property_type("住宅"));
        assert!(is_occupancy("交吉"));
        assert!(is_occupancy("連租約"));
        // Numbers / addresses are not property types.
        assert!(!is_property_type("543"));
        assert!(!is_property_type("新界元朗八鄉"));
    }
}
