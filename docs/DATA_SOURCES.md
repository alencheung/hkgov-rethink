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
- **Datasets implemented in v1**:
  - `capital-market-statistics` →
    `.../market-data-and-statistics/monthly-statistical-bulletin/financial/capital-market-statistics`
  - `daily-interbank-liquidity` →
    `.../market-data-and-statistics/daily-figures-interbank-liquidity`
- **Paging**: `?pagesize=1000` is the documented maximum.

## 2. data.gov.hk (connector stubbed — ROADMAP v2)

⚠️ **Important**: data.gov.hk does **not** expose the standard CKAN action API.
`https://data.gov.hk/api/3/action/*` returns HTTP 404. Use the platform's own
endpoints instead:

- **Filter / query a dataset**: `https://api.data.gov.hk/v2/filter?q={urlencoded-JSON}`
  - `q` shape: `{"resource":"<dataset resource URL>","section":1,"format":"json"}`
  - Verified to return a bare JSON array of row objects.
- **Historical archive listing**:
  `https://app.data.gov.hk/v1/historical-archive/list-files?start=YYYYMMDD&end=YYYYMMDD&provider=<org>&max=<n>`
- **Catalog search**: <https://data.gov.hk/en-data/dataset?publisher=hk-hkma>

## 3. HKSAR Government press releases (connector stubbed — ROADMAP v2)

- **ISD press release archive** (1997→): <https://www.info.gov.hk/gia/general/today.htm>
  (EN) / `ctoday.htm` (中文). HTML; needs scraping.
- **news.gov.hk RSS**: <https://www.news.gov.hk/eng/rss/index.html>
- **GovHK RSS hub**: <https://www.gov.hk/en/about/rss.htm>
- **HKMA press releases (API)**:
  <https://data.gov.hk/en-data/dataset/hk-hkma-pressrel-press-releases>

## 4. Geospatial (connector stubbed — ROADMAP v2)

- ⛔ **Excluded**: `api.portal.hkmapservice.gov.hk` — restricted to Government
  Departments only; no public API key. See
  <https://api.portal.hkmapservice.gov.hk/about>.
- ✅ **Used instead** (open):
  - LandsD topographic map tile API (data.gov.hk):
    <https://data.gov.hk/en-data/dataset/hk-landsd-openmap-landsd-topo-map-api>
  - CSDI 3D Visualisation Map API:
    <https://portal.csdi.gov.hk/csdi-webpage/apidoc/3d-visualisation-map-api>
  - CSDI portal (~1,100 datasets): <https://portal.csdi.gov.hk/>

## Politeness

HKGOV endpoints are free public infrastructure. Defaults are conservative
(`hkma_rate_per_sec=5`, bounded retries, gzip on). Do not raise rate limits
without a co-ordinated capacity discussion — see CONTRIBUTING (to be added).
