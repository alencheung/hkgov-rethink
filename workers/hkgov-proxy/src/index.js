/**
 * hkgov-proxy — plain-fetch Worker fronting HK property portals.
 * See README.md. No Browser Run — every dataset is reachable as either SSR
 * HTML or a clean JSON API.
 */
const ALLOWED_HOSTNAMES = new Set([
  'www.hkp.com.hk', 'www.midland.com.hk',
  'www.chungsen.com.hk', 'www.aaproperty.com.hk',
  'data.midland.com.hk', 'data.hkp.com.hk',
]);
const BROWSER_HEADERS = {
  'User-Agent': 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36',
  'Accept': 'text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8',
  'Accept-Language': 'zh-HK,zh;q=0.9,en;q=0.8',
};

export default {
  async fetch(request, _env) {
    const url = new URL(request.url);
    if (url.pathname === '/health') return json({ ok: true, ts: Date.now() });
    if (url.pathname !== '/fetch') return json({ error: 'not found' }, 404);
    if (request.method !== 'GET') return json({ error: 'method not allowed' }, 405);

    // Optional custom headers to forward upstream (e.g. Authorization for
    // Midland's data API). Pass as ?header_<name>=<value>; we forward any
    // such header verbatim, but only to allowlisted hosts.
    const fwdHeaders = {};
    for (const [k, v] of url.searchParams.entries()) {
      if (k.startsWith('header_') && v) {
        fwdHeaders[k.slice(7)] = v;
      }
    }
    const target = url.searchParams.get('url');
    if (!target) return json({ error: 'missing ?url=' }, 400);
    let targetUrl;
    try { targetUrl = new URL(target); } catch { return json({ error: 'invalid url' }, 400); }
    if (targetUrl.protocol !== 'https:' && targetUrl.protocol !== 'http:') {
      return json({ error: 'only http(s) urls are allowed' }, 400);
    }
    if (!ALLOWED_HOSTNAMES.has(targetUrl.hostname)) {
      return json({ error: 'host not allowed' }, 403);
    }
    const headers = { ...BROWSER_HEADERS, ...fwdHeaders };
    // For API hosts, prefer the JSON Accept. For HTML hosts, the BROWSER_HEADERS
    // Accept is correct.
    if (targetUrl.hostname.startsWith('data.') || targetUrl.hostname.startsWith('app2.')) {
      headers['Accept'] = 'application/json, text/plain, */*';
      headers['Referer'] = `https://www.${targetUrl.hostname.split('.').slice(-3).join('.')}/`;
      headers['Origin'] = `https://www.${targetUrl.hostname.split('.').slice(-3).join('.')}`;
    }
    try {
      const r = await fetch(targetUrl.toString(), { headers, redirect: 'follow' });
      const text = await r.text();
      return json({ status: r.status, ct: r.headers.get('content-type'), size: text.length, body: text });
    } catch (err) {
      return json({ error: 'proxy error', detail: String((err && err.message) || err) }, 502);
    }
  },
};

function json(obj, status = 200) {
  return new Response(JSON.stringify(obj), {
    status,
    headers: { 'content-type': 'application/json; charset=utf-8' },
  });
}
