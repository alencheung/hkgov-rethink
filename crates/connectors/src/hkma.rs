//! Hong Kong Monetary Authority (HKMA) Open API connector.
//!
//! Verified live against `https://api.hkma.gov.hk/public/...`. Upstream returns
//! a stable envelope:
//!
//! ```jsonc
//! {
//!   "header": { "success": true, "err_code": "0000", "err_msg": "..." },
//!   "result": { "datasize": 3, "records": [ { ...row... }, ... ] }
//! }
//! ```
//!
//! Fields per record are dataset-specific and sparse (many `null`s), so we keep
//! them as `serde_json::Value` and normalize into [`RecordValue`] cells. That
//! keeps this connector resilient when HKMA adds new columns.
//!
//! ## Dataset coverage
//!
//! The `DATASETS` table below enumerates the **entire public HKMA Open API
//! catalog** — every dataset listed under `apidocs.hkma.gov.hk/documentation`,
//! each one probe-verified live (HTTP 200 + `header.success`). That is 151
//! datasets across 14 sections:
//!
//! - Monthly Statistical Bulletin (financial, money, banking, money-markets,
//!   efbn, er-ir, monetary-operation, ef-fc-resv-assets, gov-bond)
//! - Daily Monetary Statistics
//! - Other (Exchange Fund)
//! - Bank & SVF Related Information
//! - Financial Market Infrastructure (Debt Securities Settlement System,
//!   Trade Repository)
//!
//! A handful of datasets require an extra query parameter to return data:
//! - `lang` (`=en`) — the `bank-svf-info` family rejects requests without it.
//! - `segment` — tender results, bond pricings, SVF licensees, HKTR
//!   disclosures, etc. need a segment selector (tenor / instrument / type).
//!   Each such row carries its verified default segment; the connector sends
//!   exactly one segment so a single fetch is deterministic.
//!
//! Note on DSSI: the Debt Securities Settlement System datasets live at
//! `/public/debt-securities-settlement-system/...` — NOT under
//! `financial-market-infra/` despite the docs URL. Verified live.

use crate::{Connector, DatasetSpec};
use async_trait::async_trait;
use chrono::Utc;
use hkgov_common::{
    Cadence, Category, DataSource, Error, NormalizedRecord, RecordValue, Result, UpstreamSettings,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::time::Duration;

/// One row of the verified HKMA dataset table. Adding a dataset = adding a row
/// here; the `Connector` impl derives everything else from the table.
#[derive(Debug, Clone, Copy)]
struct HkmaDataset {
    /// Dataset id (the slug used in `/v1/datasets/hkma/<slug>`).
    slug: &'static str,
    /// Human-readable title (HKMA bulletin numbering preserved).
    title: &'static str,
    /// API path after `/public/`. Verified live.
    path: &'static str,
    category: Category,
    cadence: Cadence,
    tags: &'static [&'static str],
    refresh_interval_secs: u64,
    /// Some datasets require a `segment` query param (tenor / instrument /
    /// type). When `Some`, the connector sends this exact segment value.
    segment: Option<&'static str>,
    /// The `bank-svf-info` family requires `lang=en`. When `true`, the
    /// connector appends `&lang=en`.
    lang: bool,
}

impl HkmaDataset {
    /// Render the full fetch URL (path + required query params + pagesize).
    fn url(&self, base_url: &str) -> String {
        let mut url = format!("{base_url}/{}?pagesize=1000", self.path);
        if let Some(seg) = self.segment {
            url.push_str("&segment=");
            url.push_str(seg);
        }
        if self.lang {
            url.push_str("&lang=en");
        }
        url
    }
}

/// The complete verified HKMA dataset table. See the module docs for the
/// section breakdown and the `segment`/`lang` param notes.
const DATASETS: &[HkmaDataset] = &[
    // --- msb-financial (4 datasets) ---
    HkmaDataset {
        slug: "monetary-statistics",
        title: "1.1 Monetary statistics",
        path: "market-data-and-statistics/monthly-statistical-bulletin/financial/monetary-statistics",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "financial-summary"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "banking-statistics",
        title: "1.2 Banking statistics",
        path: "market-data-and-statistics/monthly-statistical-bulletin/financial/banking-statistics",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "financial-summary"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "capital-market-statistics",
        title: "1.3 Capital market statistics",
        path: "market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "financial-summary"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "economic-statistics",
        title: "1.4 Economic statistics",
        path: "market-data-and-statistics/monthly-statistical-bulletin/financial/economic-statistics",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "financial-summary"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-money (7 datasets) ---
    HkmaDataset {
        slug: "currency",
        title: "2.1 Currency in circulation",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money/currency",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-supply"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "supply-adjusted",
        title: "2.2.1 Money supply – Adjusted for foreign currency swap deposits",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money/supply-adjusted",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-supply"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "supply-unadjusted-fc",
        title: "2.2.2 Money supply – Unadjusted for foreign currency swap deposits",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money/supply-unadjusted-fc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-supply"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "supply-components-hkd",
        title: "2.3.1 Components of money supply – Hong Kong dollar (Adjusted to include foreign currency swap deposits)",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money/supply-components-hkd",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-supply"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "supply-components-fc",
        title: "2.3.2 Components of money supply – Foreign currency (Adjusted to exclude foreign currency swap deposits)",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money/supply-components-fc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-supply"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "supply-components-all",
        title: "2.3.3 Components of money supply – All currencies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money/supply-components-all",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-supply"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "components-seasonally-adjusted-hkd",
        title: "2.4 Components of seasonally adjusted Hong Kong dollar M1",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money/components-seasonally-adjusted-hkd",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-supply"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-banking (49 datasets) ---
    HkmaDataset {
        slug: "number-of-ais-lros",
        title: "3.1 Number of authorized institutions and local representative offices",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/number-of-ais-lros",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "customer-deposits-by-currency",
        title: "3.2 Customer deposits by currency",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/customer-deposits-by-currency",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "customer-deposits-by-type-hkd-fc",
        title: "3.3.1 Customer deposits by type – Hong Kong dollar and foreign currency deposits",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/customer-deposits-by-type-hkd-fc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "customer-deposits-by-type-cny",
        title: "3.3.2 Customer deposits by type – Renminbi deposits",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/customer-deposits-by-type-cny",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-type-ais",
        title: "3.4.1 Loans and advances by type – Authorized institutions",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-type-ais",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-type-lb",
        title: "3.4.2 Loans and advances by type – Licensed banks",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-type-lb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-type-rlb",
        title: "3.4.3 Loans and advances by type – Restricted licence banks",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-type-rlb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-type-dtc",
        title: "3.4.4 Loans and advances by type – Deposit-taking companies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-type-dtc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-sector-ais",
        title: "3.5.1 Loans and advances for use in Hong Kong by economic sector – Authorized institutions",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-sector-ais",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-sector-lb",
        title: "3.5.2 Loans and advances for use in Hong Kong by economic sector – Licensed banks",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-sector-lb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-sector-rlb",
        title: "3.5.3 Loans and advances for use in Hong Kong by economic sector – Restricted licence banks",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-sector-rlb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "loans-by-sector-dtc",
        title: "3.5.4 Loans and advances for use in Hong Kong by economic sector – Deposit-taking companies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/loans-by-sector-dtc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "assetquality-ais",
        title: "3.6.1 Asset quality – Authorized institutions",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/assetquality-ais",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "assetquality-retailbanks",
        title: "3.6.2 Asset quality – Retail banks",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/assetquality-retailbanks",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "residential-mortgage-survey",
        title: "3.7 Residential mortgage survey results",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/residential-mortgage-survey",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "credit-card-lending-survey",
        title: "3.8 Credit card lending survey results",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/credit-card-lending-survey",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "balance-sheet-ais",
        title: "3.9.1 Balance sheet – Authorized institutions",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/balance-sheet-ais",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "balance-sheet-lb",
        title: "3.9.2 Balance sheet – Licensed banks",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/balance-sheet-lb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "balance-sheet-rlb",
        title: "3.9.3 Balance sheet – Restricted licence banks",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/balance-sheet-rlb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "balance-sheet-dtc",
        title: "3.9.4 Balance sheet – Deposit-taking companies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/balance-sheet-dtc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-all",
        title: "3.10.1 Foreign currency position – All foreign currencies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-all",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-usd",
        title: "3.10.2 Foreign currency position – US dollar",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-usd",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-gbp",
        title: "3.10.3 Foreign currency position – Pound sterling",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-gbp",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-jpy",
        title: "3.10.4 Foreign currency position – Japanese yen",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-jpy",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-eur",
        title: "3.10.5 Foreign currency position – Euro",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-eur",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-dem",
        title: "3.10.6 Foreign currency position – Deutschmark",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-dem",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-cad",
        title: "3.10.7 Foreign currency position – Canadian dollar",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-cad",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-chf",
        title: "3.10.8 Foreign currency position – Swiss franc",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-chf",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-position-other",
        title: "3.10.9 Foreign currency position – Other foreign currencies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/fc-position-other",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "elc-endperiod",
        title: "3.11.1 External liabilities and claims – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/elc-endperiod",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "elc-pos-v-all",
        title: "3.11.2 External liabilities and claims – Positions vis-a-vis all countries",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/elc-pos-v-all",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "elc-pos-v-mc",
        title: "3.11.3 External liabilities and claims – Positions vis-a-vis Mainland China",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/elc-pos-v-mc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "elc-pos-v-jp",
        title: "3.11.4 External liabilities and claims – Positions vis-a-vis Japan",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/elc-pos-v-jp",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "elc-pos-v-sin",
        title: "3.11.5 External liabilities and claims – Positions vis-a-vis Singapore",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/elc-pos-v-sin",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "elc-pos-v-uk",
        title: "3.11.6 External liabilities and claims – Positions vis-a-vis United Kingdom",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/elc-pos-v-uk",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "elc-pos-v-us",
        title: "3.11.7 External liabilities and claims – Positions vis-a-vis United States",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/elc-pos-v-us",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ch-statistics-ch-turnover",
        title: "3.12.1 Clearing House Statistics – Clearing House Turnover",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/ch-statistics-ch-turnover",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ch-statistics-fps-reg",
        title: "3.12.2 Clearing House Statistics – Faster Payment System Addressing Service Registrations",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/ch-statistics-fps-reg",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ch-statistics-turnover-fps-hkd-payment-vol",
        title: "3.12.3 Clearing House Statistics – Turnover of HKD FPS payment (Volume)",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/ch-statistics-turnover-fps-hkd-payment-vol",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ch-statistics-turnover-fps-hkd-payment-val",
        title: "3.12.4 Clearing House Statistics – Turnover of HKD FPS payment (Value)",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/ch-statistics-turnover-fps-hkd-payment-val",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ch-statistics-turnover-fps-rmb-payment-vol",
        title: "3.12.5 Clearing House Statistics – Turnover of RMB FPS payment (Volume)",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/ch-statistics-turnover-fps-rmb-payment-vol",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ch-statistics-turnover-fps-rmb-payment-val",
        title: "3.12.6 Clearing House Statistics – Turnover of RMB FPS payment (Value)",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/ch-statistics-turnover-fps-rmb-payment-val",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "mr-lending",
        title: "3.13.1 Mainland-related lending and other non-bank exposures – Mainland-related lending",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/mr-lending",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "mr-lending-ais-type",
        title: "3.13.2 Mainland-related lending and other non-bank exposures – Mainland-related lending by type of AIs",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/mr-lending-ais-type",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "mr-lending-borrowers-type",
        title: "3.13.3 Mainland-related lending and other non-bank exposures – Mainland-related lending by type of borrowers",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/mr-lending-borrowers-type",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "other-mr-non-bank-exposures",
        title: "3.13.4 Mainland-related lending and other non-bank exposures – Other Mainland-related non-bank exposures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/other-mr-non-bank-exposures",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "capital-adequacy",
        title: "3.14 Capital adequacy",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/capital-adequacy",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "liquidity",
        title: "3.15 Liquidity",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/liquidity",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "residential-mortgage-loans-neg-equity",
        title: "3.16 Residential Mortgage Loans in Negative Equity",
        path: "market-data-and-statistics/monthly-statistical-bulletin/banking/residential-mortgage-loans-neg-equity",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "banking"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-money-markets (6 datasets) ---
    HkmaDataset {
        slug: "hkd-interbank-trans",
        title: "4.1 Hong Kong dollar interbank transactions",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money-markets/hkd-interbank-trans",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-markets"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "liab-dt-other-ais",
        title: "4.2 Liabilities due to other authorized institutions",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money-markets/liab-dt-other-ais",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-markets"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ncds-issued-in-hk",
        title: "4.3 Analysis of NCDs issued in Hong Kong",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money-markets/ncds-issued-in-hk",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-markets"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "turnover-ncds-sec-mar-hk-ais",
        title: "4.4 Turnover of NCDs in the secondary market in Hong Kong by authorized institutions",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money-markets/turnover-ncds-sec-mar-hk-ais",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-markets"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "osamt-hkd-debtinst-otherthan-efbn",
        title: "4.5 Outstanding amount of Hong Kong dollar debt instruments, other than Exchange Fund Bills and Notes",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money-markets/osamt-hkd-debtinst-otherthan-efbn",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-markets"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ni-hkd-debt-inst-oth-efbn",
        title: "4.6 New issues of Hong Kong dollar debt instruments, other than Exchange Fund Bills and Notes",
        path: "market-data-and-statistics/monthly-statistical-bulletin/money-markets/ni-hkd-debt-inst-oth-efbn",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "money-markets"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-efbn (20 datasets) ---
    HkmaDataset {
        slug: "efbn-turnover-sec-mkt-original-maturity",
        title: "5.1.1 Turnover of Exchange Fund Bills & Notes in the secondary market – Original maturity",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-turnover-sec-mkt-original-maturity",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-turnover-sec-mkt-remaining-tenor",
        title: "5.1.2 Turnover of Exchange Fund Bills & Notes in the secondary market – Remaining tenor",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-turnover-sec-mkt-remaining-tenor",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-oustanding-original-maturity",
        title: "5.2.1 Outstanding amount of Exchange Fund Bills & Notes – Original maturity",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-oustanding-original-maturity",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-outstanding-remaining-tenor",
        title: "5.2.2 Outstanding amount of Exchange Fund Bills & Notes – Remaining tenor",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-outstanding-remaining-tenor",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-yield-endperiod",
        title: "5.3.1 Yield of Exchange Fund Bills & Notes – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-yield-endperiod",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-yield-periodaverage",
        title: "5.3.2 Yield of Exchange Fund Bills & Notes – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-yield-periodaverage",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-yield-daily",
        title: "5.3.3 Yield of Exchange Fund Bills & Notes – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-yield-daily",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-tender-results-efb",
        title: "5.4.1 Tender results of Exchange Fund Bills and Notes – Exchange Fund Bills",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-tender-results-efb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: Some("28day"),
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-tender-results-efn",
        title: "5.4.2 Tender results of Exchange Fund Bills and Notes – Exchange Fund Notes",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-tender-results-efn",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: Some("2year"),
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-service",
        title: "5.5 Central Moneymarkets Unit (CMU) service",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-service",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-turnover-sec-mkt-remain-tenor-all-currencies",
        title: "5.6.1 Turnover of CMU issues in the secondary market (remaining tenor) – All currencies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-turnover-sec-mkt-remain-tenor-all-currencies",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-turnover-sec-mkt-remain-tenor-hkd",
        title: "5.6.2 Turnover of CMU issues in the secondary market (remaining tenor) – Hong Kong dollar-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-turnover-sec-mkt-remain-tenor-hkd",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-turnover-sec-mkt-remain-tenor-rmb",
        title: "5.6.3 Turnover of CMU issues in the secondary market (remaining tenor) – Renminbi-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-turnover-sec-mkt-remain-tenor-rmb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-turnover-sec-mkt-remain-tenor-usd",
        title: "5.6.4 Turnover of CMU issues in the secondary market (remaining tenor) – US dollar-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-turnover-sec-mkt-remain-tenor-usd",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-turnover-sec-mkt-remain-tenor-other-fc",
        title: "5.6.5 Turnover of CMU issues in the secondary market (remaining tenor) – Other foreign currency-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-turnover-sec-mkt-remain-tenor-other-fc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-outstanding-remain-tenor-all-currencies",
        title: "5.7.1 Outstanding amount of CMU issues (remaining tenor) – All currencies",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-outstanding-remain-tenor-all-currencies",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-outstanding-remain-tenor-hkd",
        title: "5.7.2 Outstanding amount of CMU issues (remaining tenor) – Hong Kong dollar-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-outstanding-remain-tenor-hkd",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-outstanding-remain-tenor-rmb",
        title: "5.7.3 Outstanding amount of CMU issues (remaining tenor) – Renminbi-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-outstanding-remain-tenor-rmb",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-outstanding-remain-tenor-usd",
        title: "5.7.4 Outstanding amount of CMU issues (remaining tenor) – US dollar-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-outstanding-remain-tenor-usd",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "cmu-outstanding-remain-tenor-other-fc",
        title: "5.7.5 Outstanding amount of CMU issues (remaining tenor) – Other foreign currency-denominated",
        path: "market-data-and-statistics/monthly-statistical-bulletin/efbn/cmu-outstanding-remain-tenor-other-fc",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund-bills-notes"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-er-ir (13 datasets) ---
    HkmaDataset {
        slug: "er-eeri-endperiod",
        title: "6.1.1 Exchange rates and the effective exchange rate indices – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/er-eeri-endperiod",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "er-eeri-periodaverage",
        title: "6.1.2 Exchange rates and the effective exchange rate indices – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/er-eeri-periodaverage",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "er-eeri-daily",
        title: "6.1.3 Exchange rates and the effective exchange rate indices – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/er-eeri-daily",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hkd-fer-endperiod",
        title: "6.2.1 Hong Kong dollar forward exchange rates – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hkd-fer-endperiod",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hkd-fer-periodaverage",
        title: "6.2.2 Hong Kong dollar forward exchange rates – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hkd-fer-periodaverage",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hkd-fer-daily",
        title: "6.2.3 Hong Kong dollar forward exchange rates – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hkd-fer-daily",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hk-interbank-ir-endperiod",
        title: "6.3.1 Hong Kong Interbank Interest Rates – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hk-interbank-ir-endperiod",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates", "hibor"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hk-interbank-ir-periodaverage",
        title: "6.3.2 Hong Kong Interbank Interest Rates – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hk-interbank-ir-periodaverage",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates", "hibor"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hk-interbank-ir-daily",
        title: "6.3.3 Hong Kong Interbank Interest Rates – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hk-interbank-ir-daily",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates", "hibor"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hkd-ir-effdates",
        title: "6.4.1 Hong Kong dollar interest rates – Rates as at effective dates",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hkd-ir-effdates",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hkd-ir-periodaverage",
        title: "6.4.2 Hong Kong dollar interest rates – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/hkd-ir-periodaverage",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "composite-ir",
        title: "6.5 Composite interest rate",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/composite-ir",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "renminbi-dr",
        title: "6.6 Renminbi deposit rates",
        path: "market-data-and-statistics/monthly-statistical-bulletin/er-ir/renminbi-dr",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-rates", "interest-rates"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-monetary-operation (7 datasets) ---
    HkmaDataset {
        slug: "market-operation-periodaverage",
        title: "7.1.1 Market operation – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/monetary-operation/market-operation-periodaverage",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "monetary-operation"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "market-operation-daily",
        title: "7.1.2 Market operation – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/monetary-operation/market-operation-daily",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "monetary-operation"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "monetary-base-endperiod",
        title: "7.2.1 Monetary Base – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/monetary-operation/monetary-base-endperiod",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "monetary-operation"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "monetary-base-daily",
        title: "7.2.2 Monetary Base – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/monetary-operation/monetary-base-daily",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "monetary-operation"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "disc-win-liquid-adj-win-rates-endperiod",
        title: "7.3.1 Discount Window and Liquidity Adjustment Window rates – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/monetary-operation/disc-win-liquid-adj-win-rates-endperiod",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "monetary-operation"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "disc-win-liquid-adj-win-rates-periodaverage",
        title: "7.3.2 Discount Window and Liquidity Adjustment Window rates – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/monetary-operation/disc-win-liquid-adj-win-rates-periodaverage",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "monetary-operation"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "disc-win-liquid-adj-win-rates-daily",
        title: "7.3.3 Discount Window and Liquidity Adjustment Window rates – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/monetary-operation/disc-win-liquid-adj-win-rates-daily",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "monetary-operation"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-ef-fc-resv-assets (7 datasets) ---
    HkmaDataset {
        slug: "ef-bal-sheet-half-yearly-efbs",
        title: "8.1.1 Exchange Fund balance sheet – Half-yearly Exchange Fund balance sheet",
        path: "market-data-and-statistics/monthly-statistical-bulletin/ef-fc-resv-assets/ef-bal-sheet-half-yearly-efbs",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund", "reserves"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ef-bal-sheet-abridged",
        title: "8.1.2 Exchange Fund balance sheet – Abridged balance sheet",
        path: "market-data-and-statistics/monthly-statistical-bulletin/ef-fc-resv-assets/ef-bal-sheet-abridged",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund", "reserves"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ef-analyt-acct",
        title: "8.2 Analytical accounts of the Exchange Fund",
        path: "market-data-and-statistics/monthly-statistical-bulletin/ef-fc-resv-assets/ef-analyt-acct",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund", "reserves"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "currency-board-account",
        title: "8.3 Currency Board Account",
        path: "market-data-and-statistics/monthly-statistical-bulletin/ef-fc-resv-assets/currency-board-account",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund", "reserves"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "fc-resv-assests",
        title: "8.4 Foreign currency reserve assets",
        path: "market-data-and-statistics/monthly-statistical-bulletin/ef-fc-resv-assets/fc-resv-assests",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund", "reserves"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "analysis-fc-reserve-assets",
        title: "8.5 Analysis of foreign currency reserve assets",
        path: "market-data-and-statistics/monthly-statistical-bulletin/ef-fc-resv-assets/analysis-fc-reserve-assets",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund", "reserves"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "data-intreserve-fcliquidity",
        title: "8.6 Data template on international reserves and foreign currency liquidity",
        path: "market-data-and-statistics/monthly-statistical-bulletin/ef-fc-resv-assets/data-intreserve-fcliquidity",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "exchange-fund", "reserves"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- msb-gov-bond (10 datasets) ---
    HkmaDataset {
        slug: "sec-mar-turnover-govbonds-ibip-original-maturity",
        title: "9.1.1 Secondary market turnover of Government Bonds issued under the Institutional Bond Issuance Programme – Original maturity",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/sec-mar-turnover-govbonds-ibip-original-maturity",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "sec-mar-turnover-govbonds-ibip-remain-tenor",
        title: "9.1.2 Secondary market turnover of Government Bonds issued under the Institutional Bond Issuance Programme – Remaining tenor",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/sec-mar-turnover-govbonds-ibip-remain-tenor",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "new-issuance-amt-gov-bonds",
        title: "9.2 New issuance amount of Government Bonds",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/new-issuance-amt-gov-bonds",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "out-amt-gov-bonds-original-maturity",
        title: "9.3.1 Outstanding amount of Government Bonds – Original maturity",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/out-amt-gov-bonds-original-maturity",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "out-amt-gov-bonds-remaining-tenor",
        title: "9.3.2 Outstanding amount of Government Bonds – Remaining tenor",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/out-amt-gov-bonds-remaining-tenor",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "instit-bond-price-yield-endperiod",
        title: "9.4.1 Prices and yields of Government Bonds issued under the Institutional Bond Issuance Programme – End of period figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/instit-bond-price-yield-endperiod",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: Some("Benchmark"),
        lang: false,
    },
    HkmaDataset {
        slug: "instit-bond-price-yield-periodaverage",
        title: "9.4.2 Prices and yields of Government Bonds issued under the Institutional Bond Issuance Programme – Period average figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/instit-bond-price-yield-periodaverage",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: Some("Benchmark"),
        lang: false,
    },
    HkmaDataset {
        slug: "instit-bond-price-yield-daily",
        title: "9.4.3 Prices and yields of Government Bonds issued under the Institutional Bond Issuance Programme – Daily figures",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/instit-bond-price-yield-daily",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: Some("Benchmark"),
        lang: false,
    },
    HkmaDataset {
        slug: "tender-results-gov-bonds-ibip",
        title: "9.5 Tender results of Government Bonds issued under the Institutional Bond Issuance Programme",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/tender-results-gov-bonds-ibip",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: Some("2year"),
        lang: false,
    },
    HkmaDataset {
        slug: "list-outstanding-govbonds",
        title: "9.6 List of outstanding Government Bonds",
        path: "market-data-and-statistics/monthly-statistical-bulletin/gov-bond/list-outstanding-govbonds",
        category: Category::Fiscal,
        cadence: Cadence::Monthly,
        tags: &["monthly-bulletin", "government-bonds"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- daily-monetary-statistics (5 datasets) ---
    HkmaDataset {
        slug: "daily-figures-interbank-liquidity",
        title: "Daily Figures of Interbank Liquidity",
        path: "market-data-and-statistics/daily-monetary-statistics/daily-figures-interbank-liquidity",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["daily", "monetary", "hibor", "liquidity"],
        refresh_interval_secs: 3600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "daily-figures-monetary-base",
        title: "Daily Figures of Monetary Base",
        path: "market-data-and-statistics/daily-monetary-statistics/daily-figures-monetary-base",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["daily", "monetary"],
        refresh_interval_secs: 3600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "usage-rmb-liquidity-fac",
        title: "Usage of Renminbi Liquidity Facility",
        path: "market-data-and-statistics/daily-monetary-statistics/usage-rmb-liquidity-fac",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["daily", "monetary"],
        refresh_interval_secs: 3600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-indicative-price",
        title: "Exchange Fund Bills and Notes Indicative Pricings",
        path: "market-data-and-statistics/daily-monetary-statistics/efbn-indicative-price",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["daily", "monetary"],
        refresh_interval_secs: 3600,
        segment: Some("IndicativePrice"),
        lang: false,
    },
    HkmaDataset {
        slug: "efbn-closing",
        title: "Exchange Fund Bills and Notes Closing Reference",
        path: "market-data-and-statistics/daily-monetary-statistics/efbn-closing",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["daily", "monetary"],
        refresh_interval_secs: 3600,
        segment: Some("Bills"),
        lang: false,
    },

    // --- other (2 datasets) ---
    HkmaDataset {
        slug: "issuance-schedules",
        title: "Tentative issuance schedule for Exchange Fund Bills and Notes",
        path: "market-data-and-statistics/other/issuance-schedules",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["exchange-fund"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "ef-operating-expenses",
        title: "Operating expenses of the Exchange Fund",
        path: "market-data-and-statistics/other/ef-operating-expenses",
        category: Category::Monetary,
        cadence: Cadence::Monthly,
        tags: &["exchange-fund"],
        refresh_interval_secs: 21600,
        segment: None,
        lang: false,
    },

    // --- bank-svf-info (14 datasets; all require lang=en) ---
    HkmaDataset {
        slug: "register-ais-lros",
        title: "Register of AIs and LROs",
        path: "bank-svf-info/register-ais-lros",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "register-ais-secstaff",
        title: "Register of Securities Staff of AIs",
        path: "bank-svf-info/register-ais-secstaff",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "register-svf-licensees",
        title: "Register of SVF Licensees",
        path: "bank-svf-info/register-svf-licensees",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: Some("SVFLic"),
        lang: true,
    },
    HkmaDataset {
        slug: "acctopen-banks-contact",
        title: "Contact details of banks for account opening",
        path: "bank-svf-info/acctopen-banks-contact",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "fraudulent-bank-scams",
        title: "Fraudulent Bank Websites, Phishing E-mails and Similar Scams",
        path: "bank-svf-info/fraudulent-bank-scams",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "hotlines-auth-retailbanks-rep",
        title: "List of hotlines for authenticating the identity of callers claiming to be bank representatives",
        path: "bank-svf-info/hotlines-auth-retailbanks-rep",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "hotlines-report-loss-credit-card",
        title: "List of hotlines for reporting loss of credit card",
        path: "bank-svf-info/hotlines-report-loss-credit-card",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "bank-complaint-progress",
        title: "Progress in the handling of banking complaints by HKMA",
        path: "bank-svf-info/bank-complaint-progress",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: Some("new"),
        lang: true,
    },
    HkmaDataset {
        slug: "ai-related-trustees",
        title: "List of AI-related Trustees",
        path: "bank-svf-info/ai-related-trustees",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: Some("AI"),
        lang: true,
    },
    HkmaDataset {
        slug: "banks-branch-locator",
        title: "Information of Branches of Retail Banks",
        path: "bank-svf-info/banks-branch-locator",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "banks-atm-locator",
        title: "Information of Automated Teller Machines of Retail Banks",
        path: "bank-svf-info/banks-atm-locator",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "banks-ssm-locator",
        title: "Information of Self-Service Banking Facilities of Retail Banks",
        path: "bank-svf-info/banks-ssm-locator",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "info-for-sme-lending-services",
        title: "Contact details of banks' dedicated hotline for SME",
        path: "bank-svf-info/info-for-sme-lending-services",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },
    HkmaDataset {
        slug: "contact-for-credit-review-arrangements",
        title: "Contact details of banks' hotline for review arrangements for credit decisions",
        path: "bank-svf-info/contact-for-credit-review-arrangements",
        category: Category::Government,
        cadence: Cadence::Daily,
        tags: &["banking", "register", "svf"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: true,
    },

    // --- dssi (4 datasets; path is /debt-securities-settlement-system/, NOT /financial-market-infra/) ---
    HkmaDataset {
        slug: "list-of-exchange-fund-bills-and-notes",
        title: "List of Exchange Fund Bills and Notes",
        path: "debt-securities-settlement-system/operational-information/list-of-exchange-fund-bills-and-notes",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["cmu", "debt-securities", "settlement"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "list-of-cmu-instruments",
        title: "List of CMU Instruments",
        path: "debt-securities-settlement-system/operational-information/list-of-cmu-instruments",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["cmu", "debt-securities", "settlement"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "list-of-recognized-dealers",
        title: "List of Recognized Dealers",
        path: "debt-securities-settlement-system/operational-information/list-of-recognized-dealers",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["cmu", "debt-securities", "settlement"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "list-of-cmu-members",
        title: "List of CMU Members",
        path: "debt-securities-settlement-system/operational-information/list-of-cmu-members",
        category: Category::Monetary,
        cadence: Cadence::Daily,
        tags: &["cmu", "debt-securities", "settlement"],
        refresh_interval_secs: 86400,
        segment: None,
        lang: false,
    },

    // --- fmi-trade-repository (3 datasets) ---
    HkmaDataset {
        slug: "list-of-tr-member-with-lei",
        title: "TR Member List associated with LEI code",
        path: "financial-market-infra/trade-repository/list-of-tr-member-with-lei",
        category: Category::Monetary,
        cadence: Cadence::Weekly,
        tags: &["trade-repository", "derivatives"],
        refresh_interval_secs: 604800,
        segment: None,
        lang: false,
    },
    HkmaDataset {
        slug: "hktr-data-disclose-fx",
        title: "HKTR Data Disclosure – Foreign Exchange Derivatives",
        path: "financial-market-infra/trade-repository/hktr-data-disclose-fx",
        category: Category::Monetary,
        cadence: Cadence::Weekly,
        tags: &["trade-repository", "derivatives"],
        refresh_interval_secs: 604800,
        segment: Some("positions"),
        lang: false,
    },
    HkmaDataset {
        slug: "hktr-data-disclose-ir",
        title: "HKTR Data Disclosure – Interest Rate Derivatives",
        path: "financial-market-infra/trade-repository/hktr-data-disclose-ir",
        category: Category::Monetary,
        cadence: Cadence::Weekly,
        tags: &["trade-repository", "derivatives"],
        refresh_interval_secs: 604800,
        segment: Some("turnover"),
        lang: false,
    },
];

pub struct HkmaConnector {
    base_url: String,
    max_retries: u32,
    client: reqwest::Client,
}

impl HkmaConnector {
    pub fn new(settings: &UpstreamSettings) -> Result<Self> {
        let mut builder = reqwest::Client::builder()
            .timeout(Duration::from_millis(settings.hkma_timeout_ms))
            .gzip(true)
            .pool_max_idle_per_host(32)
            .user_agent(concat!("hkgov-rethink/", env!("CARGO_PKG_VERSION")));

        if let Some(key) = settings.hkma_api_key.as_deref() {
            let mut headers = reqwest::header::HeaderMap::new();
            if let Ok(v) = reqwest::header::HeaderValue::from_str(key) {
                headers.insert("X-API-KEY", v);
            }
            builder = builder.default_headers(headers);
        }

        let client = builder
            .build()
            .map_err(|e| Error::Internal(format!("reqwest build: {e}")))?;

        Ok(Self {
            base_url: settings.hkma_base_url.trim_end_matches('/').to_string(),
            max_retries: settings.hkma_max_retries,
            client,
        })
    }

    /// Look up a dataset row by slug. O(n) but n is small (151) and the call is
    /// cold (once per refresh interval per dataset).
    fn dataset(&self, slug: &str) -> Option<&'static HkmaDataset> {
        DATASETS.iter().find(|d| d.slug == slug)
    }

    /// Single GET with bounded exponential backoff. Retries are safe: HKMA
    /// endpoints are idempotent reads.
    async fn get_with_retry(&self, url: &str) -> Result<serde_json::Value> {
        let mut last_err: Option<Error> = None;
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let backoff = Duration::from_millis(200 * (1u64 << (attempt.min(6))));
                tokio::time::sleep(backoff).await;
            }

            tracing::debug!(attempt, url, "hkma request");
            let req = self.client.get(url);
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    last_err = Some(Error::Upstream {
                        origin: "hkma",
                        status: 0,
                        detail: format!("transport: {e}"),
                    });
                    continue;
                }
            };

            let status = resp.status().as_u16();
            if !resp.status().is_success() {
                let detail = resp.text().await.unwrap_or_default();
                last_err = Some(Error::Upstream {
                    origin: "hkma",
                    status,
                    detail,
                });
                // 4xx other than 429 won't fix themselves; stop early.
                if (400..500).contains(&status) && status != 429 {
                    break;
                }
                continue;
            }

            let json: serde_json::Value = resp.json().await.map_err(|e| Error::Decode {
                origin: "hkma",
                backtrace: serde::de::Error::custom(e.to_string()),
            })?;

            return Ok(json);
        }
        Err(last_err.unwrap_or_else(|| Error::Upstream {
            origin: "hkma",
            status: 0,
            detail: "exhausted retries".to_string(),
        }))
    }
}

/// HKMA response envelope — see module docs.
#[derive(Debug, Deserialize)]
struct HkmaEnvelope {
    header: HkmaHeader,
    result: HkmaResult,
}

#[derive(Debug, Deserialize)]
struct HkmaHeader {
    success: bool,
    #[serde(default)]
    err_code: String,
    #[serde(default)]
    err_msg: String,
}

#[derive(Debug, Deserialize)]
struct HkmaResult {
    #[serde(default)]
    datasize: u64,
    #[serde(default)]
    records: Vec<serde_json::Value>,
}

#[async_trait]
impl Connector for HkmaConnector {
    fn source(&self) -> DataSource {
        DataSource::Hkma
    }

    fn datasets(&self) -> &[DatasetSpec] {
        // The table is the source of truth; project each row onto a DatasetSpec
        // lazily on first call. The projected slice lives for the process lifetime.
        ensure_specs_initialized();
        HKMA_SPECS
            .get()
            .map(Vec::as_slice)
            .expect("HKMA specs initialized")
    }

    async fn fetch(&self, dataset: &str) -> Result<Vec<NormalizedRecord>> {
        let ds = self.dataset(dataset).ok_or_else(|| {
            Error::Internal(format!("hkma: no path mapping for dataset {dataset}"))
        })?;
        let url = ds.url(&self.base_url);

        let json = self.get_with_retry(&url).await?;
        let env: HkmaEnvelope = serde_json::from_value(json).map_err(|e| Error::Decode {
            origin: "hkma",
            backtrace: e,
        })?;

        if !env.header.success {
            return Err(Error::Upstream {
                origin: "hkma",
                status: 200,
                detail: format!("{}: {}", env.header.err_code, env.header.err_msg),
            });
        }

        let now = Utc::now();
        let records = env
            .result
            .records
            .into_iter()
            .map(|raw| {
                let fields = normalize_row(&raw);
                let record_id = record_id_for(dataset, &fields);
                NormalizedRecord {
                    source: DataSource::Hkma,
                    dataset: dataset.to_string(),
                    record_id,
                    fields,
                    fetched_at: now,
                }
            })
            .collect();

        tracing::info!(
            dataset,
            count = env.result.datasize,
            "hkma: fetched dataset"
        );
        Ok(records)
    }
}

/// Lazy-built `DatasetSpec` slice projected from [`DATASETS`]. Held in a
/// `OnceLock` so the projection happens exactly once per process and the
/// connector can hand out a `&'static`-lifetime view.
static HKMA_SPECS: std::sync::OnceLock<Vec<DatasetSpec>> = std::sync::OnceLock::new();

/// Initialize the projected specs once. Called from the registry build so the
/// `&'static` lifetime in `datasets()` is sound.
pub(crate) fn ensure_specs_initialized() {
    HKMA_SPECS.get_or_init(|| {
        DATASETS
            .iter()
            .map(|d| {
                // Description is derived from the title prefix — keeps the
                // catalog self-describing without hand-authoring 151 strings.
                let desc = format!("HKMA Open API: {}", d.title);
                DatasetSpec {
                    id: d.slug,
                    title: d.title,
                    description: Some(Box::leak(desc.into_boxed_str())),
                    category: d.category,
                    tags: d.tags,
                    cadence: d.cadence,
                    refresh_interval_secs: d.refresh_interval_secs,
                }
            })
            .collect()
    });
}

/// Convert a raw JSON object into our [`RecordValue`] map.
fn normalize_row(raw: &serde_json::Value) -> BTreeMap<String, RecordValue> {
    let Some(obj) = raw.as_object() else {
        return BTreeMap::new();
    };
    obj.iter()
        .map(|(k, v)| (k.clone(), json_to_value(v)))
        .collect()
}

fn json_to_value(v: &serde_json::Value) -> RecordValue {
    match v {
        serde_json::Value::Null => RecordValue::Null,
        serde_json::Value::Bool(b) => RecordValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                RecordValue::Int(i)
            } else if let Some(f) = n.as_f64() {
                RecordValue::Float(f)
            } else {
                RecordValue::Null
            }
        }
        serde_json::Value::String(s) => RecordValue::Str(s.clone()),
        // Flatten arrays/objects to their compact JSON string. Keeps the cell
        // scalar-friendly for downstream serialization and AI ingestion.
        other => RecordValue::Str(other.to_string()),
    }
}

/// Derive a stable per-record id. For monthly statistics HKMA keys on
/// `end_of_month`; daily ones on `date`. Fall back to a hash so we always have
/// *something* stable.
fn record_id_for(dataset: &str, fields: &BTreeMap<String, RecordValue>) -> String {
    // D-012: when the HKMA catalog was widened from 5 datasets to the full 151,
    // every new dataset fell through to the hash fallback below, producing
    // opaque `id-<hash>` record ids. That broke two things: (a) evidence
    // pointers in insights became unreadable, and (b) `cross_source_gap`
    // joins press release dates against data record_ids, so hash ids meant
    // every press date looked "unexplained". A natural date key per record
    // fixes both. The dataset-specific map covers the legacy slugs and the
    // exact period keys the detectors were authored against; the generic
    // fallback then picks up the other ~150 datasets from any date-like field
    // they carry, before hashing.
    let candidates: &[&str] = match dataset {
        "capital-market-statistics" | "residential-mortgage-survey" => &["end_of_month"],
        "daily-interbank-liquidity" | "daily-figures-interbank-liquidity" => {
            &["date", "end_of_date"]
        }
        _ => &[],
    };
    for key in candidates {
        if let Some(RecordValue::Str(s)) = fields.get(*key) {
            return s.clone();
        }
        if let Some(RecordValue::Int(i)) = fields.get(*key) {
            return i.to_string();
        }
    }
    // Generic fallback: scan for any of the common HKMA date/period field
    // names, in priority order. Almost every HKMA dataset exposes its period
    // as one of these columns. This keeps record ids human-readable and
    // (where the field is a true calendar date) joinable by cross_source_gap.
    const GENERIC_DATE_FIELDS: &[&str] = &[
        "end_of_date",
        "end_of_month",
        "end_of_quarter",
        "end_of_year",
        "date",
        "year_month",
        "quarter",
        "year",
    ];
    for key in GENERIC_DATE_FIELDS {
        if let Some(RecordValue::Str(s)) = fields.get(*key) {
            return s.clone();
        }
        if let Some(RecordValue::Int(i)) = fields.get(*key) {
            return i.to_string();
        }
    }
    // Deterministic fallback when no date/period field is present.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    for (k, v) in fields {
        k.hash(&mut h);
        format!("{v:?}").hash(&mut h);
    }
    format!("id-{:016x}", h.finish())
}

/// Public test helper: expose normalization so unit tests can assert against
/// fixture payloads without going to the network.
pub(crate) fn _test_normalize(raw: &serde_json::Value) -> BTreeMap<String, RecordValue> {
    normalize_row(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mirrors the shape of a real HKMA capital-market-statistics record
    /// (captured live, trimmed for the test).
    const SAMPLE: &str = r#"{
        "header": {"success": true, "err_code": "0000", "err_msg": "No error found"},
        "result": {
            "datasize": 1,
            "records": [
                {
                    "end_of_month": "2026-05",
                    "hkd_drmkt_outstand_efbn": 1354062,
                    "hkd_drmkt_outstand_odrinst": null,
                    "eq_mkt_hs_index": 25182.39,
                    "eq_mkt_ttl_stock_cap": 47078571.017408
                }
            ]
        }
    }"#;

    #[test]
    fn parses_envelope_and_normalizes_row() {
        let v: serde_json::Value = serde_json::from_str(SAMPLE).unwrap();
        let env: HkmaEnvelope = serde_json::from_value(v).unwrap();
        assert!(env.header.success);
        assert_eq!(env.result.datasize, 1);
        let row = &env.result.records[0];
        let fields = _test_normalize(row);
        assert_eq!(
            fields.get("end_of_month"),
            Some(&RecordValue::Str("2026-05".into()))
        );
        assert_eq!(
            fields.get("hkd_drmkt_outstand_efbn"),
            Some(&RecordValue::Int(1354062))
        );
        assert_eq!(
            fields.get("hkd_drmkt_outstand_odrinst"),
            Some(&RecordValue::Null)
        );
        // float preserved
        match fields.get("eq_mkt_hs_index") {
            Some(RecordValue::Float(f)) => assert!((f - 25182.39).abs() < 1e-6),
            other => panic!("expected float, got {other:?}"),
        }
    }

    #[test]
    fn record_id_prefers_end_of_month() {
        let mut fields = BTreeMap::new();
        fields.insert("end_of_month".into(), RecordValue::Str("2026-05".into()));
        let id = record_id_for("capital-market-statistics", &fields);
        assert_eq!(id, "2026-05");
    }

    // ---- D-012: generic date-key fallback so widened datasets get natural ids ----
    //
    // Before this fix, every dataset not in the two-row explicit map fell through
    // to the hash fallback, producing opaque `id-<hash>` ids. That broke
    // cross_source_gap (which joins press dates against data record_ids) and made
    // evidence pointers unreadable. The generic fallback now picks up any common
    // HKMA date/period field, so the ~150 datasets added in the catalog widening
    // keep human-readable, joinable ids.

    #[test]
    fn record_id_uses_end_of_date_for_daily_interbank() {
        let mut fields = BTreeMap::new();
        fields.insert("end_of_date".into(), RecordValue::Str("2026-06-18".into()));
        fields.insert("hibor_overnight".into(), RecordValue::Float(2.93));
        let id = record_id_for("daily-figures-interbank-liquidity", &fields);
        assert_eq!(
            id, "2026-06-18",
            "daily interbank id must be its date, not a hash"
        );
    }

    #[test]
    fn record_id_generic_fallback_picks_up_unknown_dataset_date_field() {
        // A dataset NOT in the explicit map should still get a natural id from
        // any date-like field it carries, rather than a hash.
        let mut fields = BTreeMap::new();
        fields.insert("end_of_quarter".into(), RecordValue::Str("2026-Q2".into()));
        fields.insert("some_metric".into(), RecordValue::Float(1.0));
        let id = record_id_for("some-new-bulletin-dataset", &fields);
        assert_eq!(
            id, "2026-Q2",
            "unknown dataset with a period field should use it"
        );
    }

    #[test]
    fn record_id_hash_fallback_only_when_no_date_field() {
        // When no date/period field is present, the deterministic hash still applies.
        let mut fields = BTreeMap::new();
        fields.insert("bank_name".into(), RecordValue::Str("ACME".into()));
        fields.insert("branch_count".into(), RecordValue::Int(42));
        let id = record_id_for("banks-branch-locator", &fields);
        assert!(id.starts_with("id-"), "no date field -> hash id, got: {id}");
        assert!(id.len() > "id-".len(), "hash id must carry a digest");
    }

    #[test]
    fn hibor_tag_present_on_interbank_datasets() {
        // D-012: the dashboard + flagship narrative rely on `?tag=hibor`
        // resolving. The tag was dropped in the catalog widening and restored.
        let hibor_tagged: Vec<_> = DATASETS
            .iter()
            .filter(|d| d.tags.contains(&"hibor"))
            .collect();
        assert!(
            !hibor_tagged.is_empty(),
            "at least one dataset must carry the hibor tag"
        );
        assert!(
            hibor_tagged
                .iter()
                .any(|d| d.slug == "daily-figures-interbank-liquidity"),
            "the daily interbank feed must be hibor-tagged"
        );
    }

    #[test]
    fn dataset_table_is_well_formed() {
        // Regression guard: the catalog must stay exhaustive and unique.
        assert_eq!(
            DATASETS.len(),
            151,
            "HKMA dataset count drifted from the verified 151"
        );
        // Every slug unique.
        let mut seen = std::collections::HashSet::new();
        for d in DATASETS {
            assert!(seen.insert(d.slug), "duplicate HKMA slug: {}", d.slug);
        }
        // Every path non-empty + starts with a known section.
        for d in DATASETS {
            assert!(!d.path.is_empty(), "empty path for {}", d.slug);
            assert!(
                d.path.starts_with("market-data-and-statistics/")
                    || d.path.starts_with("bank-svf-info/")
                    || d.path.starts_with("debt-securities-settlement-system/")
                    || d.path.starts_with("financial-market-infra/"),
                "unexpected path prefix for {}: {}",
                d.slug,
                d.path
            );
        }
    }

    #[test]
    fn url_builder_adds_segment_and_lang() {
        let plain = DATASETS
            .iter()
            .find(|d| d.slug == "capital-market-statistics")
            .unwrap();
        assert_eq!(
            plain.url("https://api.hkma.gov.hk/public"),
            "https://api.hkma.gov.hk/public/market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics?pagesize=1000"
        );

        let seg = DATASETS
            .iter()
            .find(|d| d.slug == "efbn-tender-results-efb")
            .unwrap();
        assert_eq!(
            seg.url("https://api.hkma.gov.hk/public"),
            "https://api.hkma.gov.hk/public/market-data-and-statistics/monthly-statistical-bulletin/efbn/efbn-tender-results-efb?pagesize=1000&segment=28day"
        );

        let lang_seg = DATASETS
            .iter()
            .find(|d| d.slug == "register-svf-licensees")
            .unwrap();
        assert_eq!(
            lang_seg.url("https://api.hkma.gov.hk/public"),
            "https://api.hkma.gov.hk/public/bank-svf-info/register-svf-licensees?pagesize=1000&segment=SVFLic&lang=en"
        );
    }

    #[cfg(feature = "live")]
    #[tokio::test]
    async fn live_fetch_capital_market() {
        use hkgov_common::Settings;
        let s = Settings::default();
        let c = HkmaConnector::new(&s.upstream).unwrap();
        let records = c.fetch("capital-market-statistics").await.unwrap();
        assert!(!records.is_empty(), "expected live records");
        assert!(records.iter().all(|r| r.source == DataSource::Hkma));
    }
}
