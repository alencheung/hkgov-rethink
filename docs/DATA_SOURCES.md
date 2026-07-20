# Data sources

All endpoints below were **verified live** during development (June–July 2026).
This file is the source of truth for what the connectors target; update it when
a connector adds or changes an endpoint. Eleven connectors are registered in
`crates/connectors/src/registry.rs`, each documented below:

| # | Source | Connector file | Datasets | Cadence | Rate limit |
|---|---|---|---|---|---|
| 1 | HKMA Open API | `hkma.rs` | 151 | mixed | 5 req/s |
| 2 | data.gov.hk | `datagovhk.rs` | 33 | mixed | 3 req/s |
| 3 | HKMA press releases | `press.rs` | 1 | daily | 2 req/s |
| 4 | LandsD/CSDI catalog | `landsd.rs` | 1 | daily | 1 req/s |
| 5 | Immigration Dept. | `immigration.rs` | 2 | daily | 2 req/s |
| 6 | Rating & Valuation Dept. | `rvd.rs` | 2 | monthly | 2 req/s |
| 7 | Land Registry | `landregistry.rs` | 2 | monthly | 2 req/s |
| 8 | Chung Sen (中誠地產) | `chungsen.rs` | 1 | daily | 1 req/s |
| 9 | AA Property (環亞拍賣) | `aaproperty.rs` | 2 | daily | 1 req/s |
| 10 | HKP (香港置業) | `hkp.rs` | 3 | mixed | 1 req/s (via Worker) |
| 11 | Midland (美聯物業) | `midland.rs` | 1 | daily | 0.5 req/s (via Worker) |

## 1. HKMA Open API (connector implemented — full catalog)

- **Docs**: <https://apidocs.hkma.gov.hk/>
- **Base URL**: `https://api.hkma.gov.hk/public/...`
- **Auth**: none required for public datasets; optional `X-API-KEY` for higher
  quota (set `HKGOV_UPSTREAM__HKMA_API_KEY`).
- **Envelope** (consistent across datasets):
  ```jsonc
  {
    "header": { "success": true, "err_code": "0000", "err_msg": "..." },
    "result": { "datasize": <n>, "records": [ { ... } ] }
  }
  ```
- **Paging**: `?pagesize=1000` is the documented maximum.
- **Coverage**: the connector now serves the **entire public HKMA Open API
  catalog — 151 datasets**, enumerated from `apidocs.hkma.gov.hk/documentation`
  and **every endpoint probe-verified live** (HTTP 200 + `header.success`). The
  table lives in `crates/connectors/src/hkma.rs` (`DATASETS`); adding a dataset
  is adding a row.

### Param requirements (verified)

Most datasets (127 of 151) need no special params. Two families do:

- **`lang` (`=en`)** — the `bank-svf-info` family (14 datasets: registers,
  locators, hotlines, complaint progress) rejects requests without it. The
  connector appends `&lang=en` automatically for these rows.
- **`segment`** — 13 datasets need a segment selector (tenor / instrument /
  type / status). Each such row carries its verified default segment value, and
  the connector appends `&segment=<value>` so a single fetch is deterministic:

  | dataset | segment | meaning |
  |---|---|---|
  | `efbn-tender-results-efb` | `28day` | Exchange Fund Bills tender — 91-day |
  | `efbn-tender-results-efn` | `2year` | Exchange Fund Notes tender |
  | `instit-bond-price-yield-{endperiod,periodaverage,daily}` | `Benchmark` | Government Bond benchmark series |
  | `tender-results-gov-bonds-ibip` | `2year` | iBIP 2-year tender |
  | `efbn-indicative-price` | `IndicativePrice` | EFB/N indicative price |
  | `efbn-closing` | `Bills` | EFB closing reference |
  | `register-svf-licensees` | `SVFLic` | SVF licensee register |
  | `bank-complaint-progress` | `new` | New (current) complaints |
  | `ai-related-trustees` | `AI` | AI-related trustees |
  | `hktr-data-disclose-{fx,ir}` | `positions` / `turnover` | HKTR disclosure |

### Section breakdown (151 datasets)

| Section | Path prefix | Count |
|---|---|---|
| MSB – Financial statistics summary | `monthly-statistical-bulletin/financial` | 4 |
| MSB – Money | `monthly-statistical-bulletin/money` | 7 |
| MSB – Banking | `monthly-statistical-bulletin/banking` | 49 |
| MSB – Money markets & debt instruments | `monthly-statistical-bulletin/money-markets` | 6 |
| MSB – Exchange Fund Bills & Notes | `monthly-statistical-bulletin/efbn` | 20 |
| MSB – Exchange rates & interest rates | `monthly-statistical-bulletin/er-ir` | 13 |
| MSB – Monetary market operation | `monthly-statistical-bulletin/monetary-operation` | 7 |
| MSB – Exchange Fund & FC reserve assets | `monthly-statistical-bulletin/ef-fc-resv-assets` | 7 |
| MSB – Government Bond Programme | `monthly-statistical-bulletin/gov-bond` | 10 |
| Daily Monetary Statistics | `daily-monetary-statistics` | 5 |
| Other (Exchange Fund) | `other` | 2 |
| Bank & SVF Related Information | `bank-svf-info` | 14 |
| Debt Securities Settlement System | `debt-securities-settlement-system` | 4 |
| Trade Repository | `financial-market-infra/trade-repository` | 3 |

> ⚠️ **DSSI path correction**: the Debt Securities Settlement System datasets
> (`list-of-cmu-*`, `list-of-recognized-dealers`, `list-of-exchange-fund-bills-and-notes`)
> live at `/public/debt-securities-settlement-system/...` — **not** under
> `financial-market-infra/` despite the docs URL. The connector uses the
> verified working path.

## 2. data.gov.hk (connector implemented — v2 filter + historical archive)

⚠️ **Important**: data.gov.hk does **not** expose the standard CKAN action API.
`https://data.gov.hk/api/3/action/*` returns HTTP 404. Use the platform's own
endpoints instead:

- **Filter / query a dataset**: `https://api.data.gov.hk/v2/filter?q={urlencoded-JSON}`
  - `q` shape: `{"resource":"<dataset resource URL>","section":1,"format":"json"}`
  - Verified to return a bare JSON array of row objects.
- **Historical archive listing**:
  `https://app.data.gov.hk/v1/historical-archive/list-files?start=YYYYMMDD&end=YYYYMMDD&provider=<org>&max=<n>`
- **Catalog search**: <https://data.gov.hk/en-data/dataset?publisher=hk-hkma>

### Resource coverage (registered subset)

The historical archive lists **376 datasets across 17 providers**, but the v2
filter API only accepts a **registered subset** of PSI resource URLs — the rest
are rejected with `{"code":"422","message":"Not a valid resource"}`. The
connector (`crates/connectors/src/datagovhk.rs`, `RESOURCES` table) registers
**every resource URL that was probe-verified live** against the filter API (HTTP
200 with a non-empty row array). As of this writing that is **33 resources**
across 8 providers:

| Provider | Department | Count | Category |
|---|---|---|---|
| `hk-cr` | Companies Registry | 11 | Fiscal |
| `hk-csd` | Correctional Services | 6 | Government |
| `hk-dh` | Dept. of Health / CHP | 5 | Livability |
| `hk-ofca` | Office of the Comm. Authority | 4 | Government |
| `hk-edb` | Education Bureau | 3 | Population |
| `hktramways` | Hong Kong Tramways | 2 | Livability |
| `hk-wsd` | Water Supplies Department | 1 | Livability |
| `centaline` | Centaline (property) | 1 | Property |

The registered PSI paths follow predictable host conventions
(`/datagovhk/psi/...`, `/files/misc/...`, `static.data.gov.hk/tramways/...`,
`centanet.com/opendata/...`, `/filemanager/ofca/...`). The full 376-dataset
catalog remains discoverable via the `landsd-catalog` connector's archive
listing; only resources that return queryable data are wired to the filter
connector.

## 3. HKSAR Government press releases (connector implemented — HKMA press API)

- **HKMA press releases API** (implemented, verified live):
  `GET api.hkma.gov.hk/public/press-releases?lang=en&pagesize=N`
  → `{header, result:{records:[{title, link, date}]}}`. Requires `lang=en|tc|sc`.
- **ISD press release archive** (1997→, future — needs HTML scraping):
  <https://www.info.gov.hk/gia/general/today.htm> (EN) / `ctoday.htm` (中文).
- **news.gov.hk RSS** (future): <https://www.news.gov.hk/eng/rss/index.html>
- **GovHK RSS hub** (future): <https://www.gov.hk/en/about/rss.htm>

## 4. Geospatial (connector implemented — open catalog)

- ⛔ **Excluded**: `api.portal.hkmapservice.gov.hk` — restricted to Government
  Departments only; no public API key. See
  <https://api.portal.hkmapservice.gov.hk/about>.
- ✅ **Implemented** (open): the `landsd-catalog` dataset lists the available
  open LandsD datasets via the data.gov.hk historical archive
  (`app.data.gov.hk/v1/historical-archive/list-files?provider=hk-landsd`).
  Live-verified: returns ~500 LandsD dataset files. The `end` param must be
  ≤ yesterday or the API rejects it.
- Future: direct LandsD tile/CSDI dataset fetch via the filter API (each
  resource must be probe-verified first — many URLs are not registered).

## 5. Immigration Department (connector implemented — daily passenger-traffic CSV)

- **Source**: Immigration Department (入境事務處) — border-crossing traffic.
- **Base URL**:
  `https://www.immd.gov.hk/opendata/eng/transport/immigration_clearance/statistics_on_daily_passenger_traffic.csv`
- **Auth**: none. The file is a plain-text CSV published daily since 2021.
- **Format**: **CSV, not a queryable API.** data.gov.hk badges it "API" but
  that refers to CKAN catalog metadata only; there is no JSON/filter endpoint.
  The connector pulls the whole file and parses it client-side
  (`crates/connectors/src/immigration.rs`).
- **Schema** (one row per `Date × Control Point × Direction`):
  ```
  Date, Control Point, Arrival / Departure,
  Hong Kong Residents, Mainland Visitors, Other Visitors, Total [, trailing empty]
  ```
- **Datasets** (2):
  - **`daily-passenger-traffic`** — the full tidy breakdown (one record per
    checkpoint × direction × day). Fields: `control_point`, `direction`,
    `hk_residents`, `mainland_visitors`, `other_visitors`, `total`.
    `record_id` = ISO date (`YYYY-MM-DD`).
  - **`daily-passenger-traffic-totals`** — one record per day, aggregated
    across all control points. Fields: `arrivals`, `departures`,
    `hk_residents`, `mainland_visitors`, `other_visitors`, `total`. This is
    the series `series_jump` runs on (a halving/doubling of cross-border flow
    is a headline opacity signal — e.g. a checkpoint quietly closed).

- **Quirks handled** (verified against the live file):
  - Dates are `DD-MM-YYYY` (not ISO) → parsed to `YYYY-MM-DD` for `record_id`.
  - A trailing empty column on every row (extra comma in the header).
  - Long/tidy format: ~28 rows per day (14 checkpoints × 2 directions).
  - Counts may carry thousands commas (`"1,186"`) → parsed to ints.

## 6. Rating & Valuation Department (connector implemented — price/rental index CSVs)

- **Source**: Rating & Valuation Department (差餉物業估價處, RVD) — property
  price/rental indices.
- **Base URLs** (verified live at `https://www.rvd.gov.hk/datagovhk/`):
  - `1.4M.csv` — Private Domestic **Price** Indices by Class (territory-wide).
  - `1.3M.csv` — Private Domestic **Rental** Indices by Class (territory-wide).
- **Auth**: none. Both files are plain-text CSVs.
- **Format**: **CSV, not a queryable API.** The connector pulls the whole file
  and parses it client-side (`crates/connectors/src/rvd.rs`).
- **Schema**: one row per month from 1993 onward. The classes are flats by
  saleable area: A (<40 m²), B (40–69.9), C (70–99.9), D (100–159.9), E (≥160),
  plus the A–B–C and D–E groupings and an All-Classes total. These feed the
  property Silence Index (a 10% monthly index move is large for a smoothed
  property series and worth flagging).
- **Datasets** (2):
  - **`price-indices-monthly`** — the headline HK property-price series.
    Fields: `class_a`..`class_e`, `classes_abc`, `classes_de`, `all_classes`.
    `record_id` = ISO month (`YYYY-MM`).
  - **`rental-indices-monthly`** — the by-class rental breakdown. Same field
    shape as the price dataset.

- **Quirks handled** (verified against the live file):
  - `Month` is `MM-YYYY` (e.g. `05-2026`) → converted to ISO `YYYY-MM` for the
    `record_id`.
  - Value columns are **interleaved** with `<Class> - Remarks` columns: every
    class has a value cell immediately followed by a remarks cell. The parser
    reads by fixed column position (indices 1, 3, 5, 7, 9, 11, 13, 15) and
    ignores the remarks columns entirely.
  - Cells may be empty or carry footnote markers → parsed defensively (non-
    numeric values are skipped, never crash; the field is omitted from the
    record rather than emitted as null).

## 7. Land Registry (connector implemented — monthly transaction JSON files)

- **Source**: Land Registry (土地註冊處) — property transactions.
- **Base URL**: `https://www.landreg.gov.hk/datagovhk/` (verified live).
- **Auth**: none. Both files are plain JSON.
- **Format**: **Plain JSON files, not a queryable API.** The connector fetches
  the current year's / current month's file(s) and parses them client-side
  (`crates/connectors/src/landregistry.rs`).
- **Datasets** (2):
  - **`monthly-transactions`** — number of S&P agreements by price band. The
    upstream file is `consideration_YYYY.json` (wide: one row per price band,
    columns = months Jan..Dec). The connector transposes it into one record
    per month with a `total_units` field (the sum across price bands) so
    `series_jump` has a clean monthly series. `record_id` = `YYYY-MM`.
  - **`monthly-primary-secondary`** — all-instruments monthly statistics
    (file `YYYYMM_data.json`), including the Primary Sales vs Secondary Sales
    split for residential units (covers 二手房 / secondary-market
    transactions). One record per month. Fields: `primary_sales`,
    `secondary_sales`. The connector tries the current month, then falls back
    to the prior month if the current file isn't published yet.

- **Quirks handled** (verified against the live files):
  - The JSON files are published with a **UTF-8 BOM** prefix. `serde_json`
    rejects BOM-prefixed input, so the connector reads as text and strips the
    BOM before parsing (`strip_bom` in `crates/connectors/src/lib.rs`). This
    was a real bug — without the strip, Land Registry showed 0 records and
    the agent's scan-readiness wait timed out at 180s.
  - Counts are strings with thousands commas (`"1,186"`) → parsed to ints.
  - The all-instruments file uses a `Description` string to name each row
    (e.g. `"Number of Secondary Sales for ASP Residential Building Units"`);
    the connector filters for the Primary/Secondary Sales rows by substring.

## 8–11. Commercial property portals (v3)

Four new sources added in v3 — private HK property-portal scrapers, distinct
from the seven government open-data connectors above. They cover the 銀主盤
(bank-owned / foreclosure) and auction-listing pools that the gov sources
don't surface.

> ⚠️ **Brittleness disclaimer.** These are commercial sites with no public
> SLA. Parsers may break when the sites change their HTML/JSON shape. Each
> connector sits behind the same circuit breaker as the gov sources, so a
> parser break → breaker trips → `/health/sources` flags the source red →
> operator sees it. Fixing a broken selector is a 10-minute patch to one
> file. This is the inherent cost of scraping commercial portals.

### The Cloudflare Worker proxy

Two of the four portals (`www.hkp.com.hk`, `www.midland.com.hk` + its API
host `data.midland.com.hk`) sit behind CloudFront WAFs that **geo-block
non-HK IPs** — the backend's default US egress gets HTTP 403 on even their
homepages. A Cloudflare Worker (`workers/hkgov-proxy/`) fronts them from
Cloudflare's edge, which the WAF accepts.

The Worker is a hard runtime dependency for the HKP + Midland connectors.
When it isn't configured (the default), those two connectors self-disable:
their datasets stay out of `/sources` and the ingest scheduler skips them,
so a default-config boot doesn't 502 every refresh. To enable them, deploy
the Worker (see `workers/hkgov-proxy/README.md`) and set the three env vars:

```
HKGOV_UPSTREAM__PROXY_URL=https://hkgov-proxy.<your-sub>.workers.dev
HKGOV_UPSTREAM__PROXY_CF_ACCESS_CLIENT_ID=<service-token-id>
HKGOV_UPSTREAM__PROXY_CF_ACCESS_CLIENT_SECRET=<service-token-secret>
```

All three must be set together or all left empty (a half-configured proxy
fails loudly at boot — see `Settings::validate`). Auth is layered: a
Cloudflare Access service token gates the Worker route, plus a hardcoded
host allowlist inside the Worker.

### 8. Chung Sen Property Group (中誠地產)

- **Source**: Chung Sen Property Group — 筍盤推介 (hot picks) + 銀主/獨家
  (bank-owned/exclusive) auction listings.
- **Base URL**: `https://www.chungsen.com.hk/tc/mortgage_property.php?wid=<id>`
  (verified live; reachable from any egress — no proxy needed).
- **Auth**: none. Plain HTML.
- **Format**: server-rendered HTML `<table>`. The connector parses the rows
  client-side.
- **Datasets** (1):
  - **`chungsen-listings`** — combined pool from both `wid=91` (筍盤推介) and
    `wid=88` (銀主/獨家). **Quirk (verified July 2026):** both wid values
    return the SAME 151-row table — the param changes only the page title.
    The connector fetches both pages (so the operator-applied labels are
    observed) and dedupes by 物業編號; a listing appearing under both is
    emitted once with `page_label = "筍盤推介; 銀主/獨家"`. Fields: `address`
    (Chinese), `build_area_sqft`, `saleable_area_sqft`, `price_10k` (售價萬,
    may be multi-track for HOS units — kept as string when so),
    `page_label`, `source_url`. `record_id` = 物業編號 (e.g. `260612-01`).

### 9. AA Property Auctioneers (環亞物業拍賣)

- **Source**: AA Property (環亞物業拍賣) — open public auction lot list.
- **Base URL**: `https://www.aaproperty.com.hk/aa/bid_list.php` (verified
  live; direct fetch).
- **Auth**: none. Plain HTML.
- **Format**: legacy HTML 4 — `<TR>` rows with `<TD>` cells. Each lot row
  carries a link to `item.php?item_no=…` (the per-lot detail page).
- **Datasets** (2):
  - **`aaproperty-auction-list`** — one record per lot. Fields: `lot_no`,
    `address`, `property_type` (空地/住宅/工商/…), `occupancy`
    (交吉/連租約/…), `area_sqft`, `price_hint` (often 歡迎查詢 = on enquiry),
    `agent_phone`, `source_url`. `record_id` = 物業編號, or `lot-<n>` when
    no id is present (raw-land lots often don't carry one).
  - **`aaproperty-auction-sessions`** — upcoming auction sessions parsed
    from the page banner. One record per session. Fields: `date`, `time`,
    `venue`. `record_id` = ISO date (`YYYY-MM-DD`). Lets the agent layer
    join "next auction date" to lot counts and address clusters.

### 10. Hong Kong Property / 香港置業 (HKP)

- **Source**: HKP — 二手樓價指數 (secondary-market price index) + economic
  indicators + 12-month Land Registry registration summary.
- **Base URLs** (verified live via the Worker):
  - `https://www.hkp.com.hk/zh-hk/market-insight` — Next.js SSR; full data
    in `<script id="__NEXT_DATA__">` JSON island.
  - `https://www.hkp.com.hk/land-registry-record/12months.html` — plain
    HTML tables.
- **Auth**: none on the HTML pages themselves (the Worker adds browser-like
  headers).
- **Format**: SSR JSON + HTML tables. The connector extracts the
  `__NEXT_DATA__` JSON via regex and deserializes into typed structs. No
  browser rendering needed.
- **Datasets** (3):
  - **`hkp-price-index-monthly`** — the 二手樓價指數. ~355 monthly points
    from 1997. Fields: `mr_index` + per-region (`_hk`/`_kln`/`_nt`)
    variants, `tx_count_*`, `net_ft_price_*`, `ft_price`, `ft_rent`,
    `monthly_perc_*`. `record_id` = `YYYY-MM`.
  - **`hkp-economic-indicators-monthly`** — ~354 monthly points from 1997.
    Fields: `mortgage_interest_rate`, `rental_yield`,
    `real_saving_interest_rate`, `hang_seng_index`, `us_dollar_index`,
    `unemployment_rate`, `affordability_ratio`,
    `rental_affordability_ratio`, `house_price_to_income_ratio`.
  - **`hkp-land-registry-summary-monthly`** — latest-month breakdown by
    property class (firsthand_private, secondhand_private, firsthand_hos,
    industrial, commercial, shop) with `{number, amount, number_chg,
    amount_chg}` per class. Augmented with rolling 12-month territory-wide
    totals from the HTML page (overall_units, overall_amount_hkd_bn).

### 11. Midland Realty (美聯物業) — 銀主盤

- **Source**: Midland — 銀主盤 (bank-owned / foreclosure) listings.
- **Base URLs** (verified live via the Worker):
  - `https://www.midland.com.hk/zh-hk/list/buy/搜尋-H-3b9d6de8` — Next.js
    SPA shell. We don't render the SPA; we extract the build-embedded
    `BUILD_TOKEN` JWT from its `__NEXT_DATA__`.
  - `https://data.midland.com.hk/search/v2/properties` — the JSON API the
    SPA itself calls. Returns full structured listing data when called with
    `Authorization: Bearer <BUILD_TOKEN>` +
    `?q=3b9d6de8&tx_type=S&category=foreclosure`.
- **Auth**: `BUILD_TOKEN` — a build-embedded JWT (issued 2020, no expiry)
  carried in the SPA shell's `__NEXT_DATA__.runtimeConfig.BUILD_TOKEN`.
  Same for every visitor. The connector fetches the shell once to extract
  it, then uses it for the search API. (An earlier version of this
  connector used Puppeteer + stealth to render the SPA — retired after a
  diagnostic showed the listing grid XHR silently returns empty for
  headless Chrome, but succeeds for direct API calls. Hitting the API
  directly is faster, cheaper, and more robust.)
- **Format**: JSON.
- **Datasets** (1):
  - **`midland-bank-listings`** — active 銀主盤 pool. The connector
    paginates through all results (24 per page). Fields: `estate_name`,
    `region`, `subregion`, `address`, `build_area_sqft`, `net_area_sqft`,
    `sale_price_hkd`, `rent_hkd`, `price_per_net_sqft`, `bedroom`,
    `tx_type`, `is_foreclosure` (bool), `tags`, `source_url`. `record_id`
    = Midland listing id (e.g. `M350591670`). ~42 銀主盤 in the pool as of
    July 2026.

## Politeness

HKGOV endpoints are free public infrastructure. Defaults are conservative and
enforced by per-source token-bucket rate limiters + circuit breakers (see
`crates/connectors/src/resilience.rs`):

| Source | Rate limit | Circuit breaker (failures → open) | Cooldown |
|---|---|---|---|
| HKMA | 5 req/s (configurable via `upstream.hkma_rate_per_sec`) | 5 | 30s |
| data.gov.hk | 3 req/s | 5 | 60s |
| press | 2 req/s | 5 | 60s |
| LandsD | 1 req/s | 3 | 120s |
| Immigration | 2 req/s | 5 | 60s |
| RVD | 2 req/s | 5 | 60s |
| Land Registry | 2 req/s | 5 | 60s |
| Chung Sen | 1 req/s | 3 | 120s |
| AA Property | 1 req/s | 3 | 120s |
| HKP | 1 req/s | 3 | 120s (via Worker) |
| Midland | 0.5 req/s | 3 | 300s (via Worker; paginated, slow) |

Do not raise these without coordination.

