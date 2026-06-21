// k6 load test for hkgov-api.
//
// Run:  k6 run loadtest/loadtest.js
// Point at a host with K6_BASE_URL, e.g.
//   K6_BASE_URL=http://localhost:8080 k6 run loadtest/loadtest.js
//
// The scenario ramps to the configured VU ceiling and holds, mixing cached reads
// (hot path) with the heavier paginated records endpoint. Use it to find the
// single-node concurrency ceiling before going multi-node (see
// docs/CAPACITY.md).

import http from 'k6/http';
import { check, sleep } from 'k6';
import { Rate } from 'k6/metrics';

const BASE = __ENV.K6_BASE_URL || 'http://localhost:8080';
const PREFIX = __ENV.K6_API_PREFIX || '/v1';
const API_KEY = __ENV.K6_API_KEY || '';
const STAGES = JSON.parse(__ENV.K6_STAGES || '[{"duration":"30s","target":500},{"duration":"1m","target":500},{"duration":"30s","target":0}]');

const errorRate = new Rate('errors');

export const options = {
  stages: STAGES,
  thresholds: {
    http_req_duration: ['p(95)<500', 'p(99)<1500'],
    errors: ['rate<0.01'],
  },
};

function headers() {
  const h = { Accept: 'application/json' };
  if (API_KEY) h['X-API-Key'] = API_KEY;
  return { headers: h };
}

export default function () {
  // 70% hot liveness + sources, 30% paginated records + insights.
  const r = Math.random();
  let res;
  if (r < 0.4) {
    res = http.get(`${BASE}/health`, headers());
  } else if (r < 0.7) {
    res = http.get(`${BASE}${PREFIX}/sources`, headers());
  } else if (r < 0.9) {
    res = http.get(`${BASE}${PREFIX}/datasets/hkma/capital-market-statistics/records?limit=20`, headers());
  } else {
    res = http.get(`${BASE}${PREFIX}/insights?limit=10`, headers());
  }

  const ok = check(res, {
    'status is 2xx': (r) => r.status >= 200 && r.status < 300,
    'has body': (r) => r.body && r.body.length > 0,
  });
  errorRate.add(!ok);

  sleep(0.05);
}

export function handleSummary(data) {
  // Print a compact capacity summary.
  const p95 = data.metrics.http_req_duration ? data.metrics.http_req_duration['p(95)'] : 'n/a';
  const rps = data.metrics.http_reqs ? data.metrics.http_reqs.rate : 'n/a';
  console.log(`\n=== CAPACITY SUMMARY ===`);
  console.log(`requests/sec: ${rps}`);
  console.log(`p95 latency: ${p95}ms`);
  console.log(`error rate: ${data.metrics.errors ? data.metrics.errors.rate : 'n/a'}`);
  return {};
}
