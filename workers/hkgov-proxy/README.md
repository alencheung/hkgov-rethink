# hkgov-proxy

Cloudflare Worker that fronts Hong Kong property portals the
`hkgov-rethink` backend cannot reach directly:

| Portal | Why proxied |
|---|---|
| `www.hkp.com.hk` + `app2.hkp.com.hk` | CloudFront WAF geo-blocks non-HK IPs → HTTP 403 from our US egress |
| `www.midland.com.hk` + `data.midland.com.hk` | Same CloudFront geo-block |
| `www.chungsen.com.hk` | Reachable directly; proxied for uniformity (one fetch path) |
| `www.aaproperty.com.hk` | Reachable directly; proxied for uniformity |

One endpoint, `GET /fetch?url=<encoded>`, plain `fetch()` with browser-like
headers. **No Browser Run / Puppeteer** — every dataset we need is reachable
as either server-rendered HTML or a clean JSON API backing the SPAs. The
connectors hit those endpoints directly through this proxy.

(An earlier version of this Worker used Puppeteer + stealth to render the
Midland + HKP SPA listing pages. That was retired after a diagnostic showed
the SPA listing grids fail to hydrate under headless Chrome — but the XHR
they fire returns full data. Hitting the XHR endpoint directly is faster,
cheaper, and more robust than rendering the SPA shell.)

## Auth model (layered)

1. **Cloudflare Access service token.** Create in Zero Trust → Access →
   Service Tokens, then create an Application for this Worker's hostname
   with a policy requiring that service token. The Rust backend sends the
   token via `CF-Access-Client-Id` + `CF-Access-Client-Secret` headers,
   read from `HKGOV_UPSTREAM__PROXY_CF_ACCESS_CLIENT_ID` and
   `HKGOV_UPSTREAM__PROXY_CF_ACCESS_CLIENT_SECRET`. No token → 403 from
   Cloudflare Access before the Worker even runs.
2. **Host allowlist** (`ALLOWED_HOSTNAMES` in `src/index.js`). Defense in
   depth: even if the service token leaks, the Worker refuses any host
   outside the six hardcoded property-portal hosts.

## Deploy

```sh
cd workers/hkgov-proxy
npm install
CLOUDFLARE_API_TOKEN=<scoped-token> npx wrangler deploy
```

Prerequisites:

- `wrangler` authenticated. Either `npx wrangler login` (interactive) or a
  scoped API token in `CLOUDFLARE_API_TOKEN` + `account_id` in
  `wrangler.toml` (non-interactive, what this project uses).

After the first deploy:

1. In the dashboard, create a Service Token under
   **Zero Trust → Access → Service Tokens**.
2. Create an Application for the Worker's URL (`hkgov-proxy.<sub>.workers.dev`
   during bring-up, or a custom hostname for production) with a policy that
   requires the service token.
3. Set the three env vars on the Rust backend:
   ```sh
   export HKGOV_UPSTREAM__PROXY_URL=https://hkgov-proxy.<sub>.workers.dev
   export HKGOV_UPSTREAM__PROXY_CF_ACCESS_CLIENT_ID=<service-token-id>
   export HKGOV_UPSTREAM__PROXY_CF_ACCESS_CLIENT_SECRET=<service-token-secret>
   ```

## Verify

```sh
# Health (no auth, no upstream call)
curl https://hkgov-proxy.<sub>.workers.dev/health

# HKP market-insight (SSR HTML — the price-index data is in __NEXT_DATA__)
curl -H "CF-Access-Client-Id: $ID" -H "CF-Access-Client-Secret: $SECRET" \
  "https://hkgov-proxy.<sub>.workers.dev/fetch?url=$(python -c 'import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=""))' \
    'https://www.hkp.com.hk/zh-hk/market-insight')"

# Midland 銀主盤 (direct JSON API — what the SPA itself calls)
curl -H "CF-Access-Client-Id: $ID" -H "CF-Access-Client-Secret: $SECRET" \
  "https://hkgov-proxy.<sub>.workers.dev/fetch?url=$(python -c 'import urllib.parse,sys;print(urllib.parse.quote(sys.argv[1],safe=""))' \
    'https://data.midland.com.hk/search/v2/properties?q=3b9d6de8&ad=true&lang=zh-hk&currency=HKD&unit=feet&search_behavior=normal&tx_type=S&category=foreclosure&limit=24')"
```

## Response shape

200 — JSON envelope:
```json
{ "status": 200, "ct": "application/json", "size": 12345, "body": "..." }
```

Non-2xx is itself an envelope error (Worker problem, not upstream problem):
```json
{ "error": "proxy error", "detail": "..." }    // 502
{ "error": "host not allowed" }                 // 403
```

Upstream non-2xx (e.g. the property portal is down) is reported inside the
200 envelope with its original status code, so the caller can distinguish
"Worker OK, upstream 403" from "Worker broken".
