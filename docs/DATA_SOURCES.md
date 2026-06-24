# Data sources

All endpoints below were **verified live** during v1 development (June 2026).
This file is the source of truth for what the connectors target; update it when
a connector adds or changes an endpoint.

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

## Politeness

HKGOV endpoints are free public infrastructure. Defaults are conservative and
enforced by per-source token-bucket rate limiters + circuit breakers (see
`crates/connectors/src/resilience.rs`): HKMA 5 req/s, data.gov.hk 3 req/s,
press 2 req/s, LandsD 1 req/s. Do not raise these without coordination.

