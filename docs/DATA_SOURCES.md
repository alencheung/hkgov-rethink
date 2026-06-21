# Data sources

All endpoints below were **verified live** during v1 development (June 2026).
This file is the source of truth for what the connectors target; update it when
a connector adds or changes an endpoint.

## 1. HKMA Open API (v1 connector — implemented)

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
- **Datasets implemented**:
  - `capital-market-statistics` →
    `.../market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics`
  - `daily-interbank-liquidity` →
    `.../market-data-and-statistics/daily-monetary-statistics/daily-figures-interbank-liquidity`
- **Paging**: `?pagesize=1000` is the documented maximum.

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

