    // ============ connection + persistence ============
    const LS_BASE='hkgov.base', LS_KEY='hkgov.key', LS_LAST='hkgov.lastvisit', LS_SEEN='hkgov.seenids', LS_LANG='hkgov.lang', LS_ONBOARD='hkgov.onboard.dismissed', LS_SESSION='hkgov.session', LS_USER='hkgov.user';
    // Default upstream API origin. D-027 fix: empty = same-origin, which is
    // correct whenever hkgov-api serves the dashboard itself (local dev, the
    // Docker image, or a Railway/cloud deploy visited directly at its own
    // host). The boot script pre-fills the base input with the page's own
    // origin, so this constant is only a last-resort fallback for an empty
    // input. For the SPLIT deploy (dashboard on Netlify, API elsewhere) set
    // this to the API's public origin (no trailing slash), e.g.
    //   const DEFAULT_API_BASE = 'https://hkgov-rethink.up.railway.app';
    // A user's saved base (localStorage) or the header input always wins.
    // Leaving it empty avoids silently pointing a self-served dashboard at a
    // hard-coded third-party host.
    const DEFAULT_API_BASE = '';
    function api() {
      // Empty base = same-origin (the deployed dashboard's own host). Override
      // via the header input to point at a separately-hosted hkgov-api instance.
      let base = (document.getElementById('baseUrl').value || DEFAULT_API_BASE).replace(/\/$/, '');
      const key = document.getElementById('apiKey').value;
      // D-018/D-033: attach the saved session as a Bearer header so per-user
      // routes (signals, investigations, silence-watch) work from the browser.
      const session = sessionStorage.getItem(LS_SESSION) || localStorage.getItem(LS_SESSION) || '';
      const headers = {};
      if (key) headers['X-API-Key'] = key;
      if (session) headers['Authorization'] = 'Bearer ' + session;
      return { base, headers };
    }
    function persistConfig() { try { localStorage.setItem(LS_BASE, document.getElementById('baseUrl').value); localStorage.setItem(LS_KEY, document.getElementById('apiKey').value); } catch(e){} }
    function restoreConfig() { try { const b=localStorage.getItem(LS_BASE); if(b) document.getElementById('baseUrl').value=b; else if(DEFAULT_API_BASE) document.getElementById('baseUrl').value=DEFAULT_API_BASE; const k=localStorage.getItem(LS_KEY); if(k) document.getElementById('apiKey').value=k; } catch(e){} }
    function setConnection(ok, partial) { const dot=document.getElementById('statusDot'); dot.className='status-dot '+(partial?'warn':(ok?'ok':'err')); dot.title=partial?'reachable — some sources degraded':(ok?'connected':'cannot reach API'); }
    async function getJSON(path) { const {base,headers}=api(); try { const r=await fetch(base+path,{headers}); setConnection(r.ok); if(!r.ok) return {__error:r.status}; return await r.json(); } catch(e){ setConnection(false); return {__error:0}; } }
    async function postJSON(path, body) { const {base,headers}=api(); try { const r=await fetch(base+path,{method:'POST',headers:{'Content-Type':'application/json',...headers},body:JSON.stringify(body)}); setConnection(r.ok); if(!r.ok) return {__error:r.status,__text:await r.text()}; return await r.json(); } catch(e){ setConnection(false); return {__error:0}; } }
    async function fetchText(path) { const {base,headers}=api(); try { const r=await fetch(base+path,{headers}); return r.ok?await r.text():null; } catch(e){ return null; } }
    function escapeHtml(s){ return String(s==null?'':s).replace(/[&<>"']/g,c=>({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c])); }
    function relTime(iso){ if(!iso) return '—'; const d=new Date(iso), now=new Date(), ms=now-d; const mins=Math.floor(ms/60000); if(mins<1) return 'just now'; if(mins<60) return mins+'m ago'; const hrs=Math.floor(mins/60); if(hrs<24) return hrs+'h ago'; return Math.floor(hrs/24)+'d ago'; }
    function cssEsc(id){ return String(id).replace(/[^a-zA-Z0-9_-]/g, m=>'_'+m.charCodeAt(0)); }

    const SEV_ICON={critical:'<i class="ri-error-warning-fill" style="color:var(--crit)"></i>',warning:'<i class="ri-alert-line" style="color:var(--warn)"></i>',info:'<i class="ri-information-line" style="color:var(--info)"></i>'};
    const SEV_BADGE={critical:'badge-crit',warning:'badge-warn',info:'badge-info'};
    const CAT_COLORS=['monetary','fiscal','property','trade','population','livability','government','other'];
    function catColorClass(cat){ return CAT_COLORS.includes(cat)?'cat-'+cat:'cat-other'; }
    // Translate a category value via the cat_<val> keys; falls back to the raw
    // value for any category not in the known taxonomy.
    function catLabel(cat){ const k='cat_'+(cat||'other'); const v=t(k); return v===k?(cat||'other'):v; }
    function catBadge(cat){ return `<span class="cat-badge" style="background:var(--${catColorClass(cat)})">${escapeHtml(catLabel(cat))}</span>`; }
    // Render a raw `source` slug (e.g. `hkma`, `landregistry`) as a human
    // institution/portal name. The source dropdowns previously showed the raw
    // enum tag; this translates via the `src_<slug>` i18n namespace and falls
    // back to a humanized form (replace `_`/`-` with spaces, capitalized) for
    // any source not yet in the dictionary.
    function sourceLabel(src){
      if(!src) return '';
      const k='src_'+String(src).toLowerCase();
      const v=t(k);
      if(v!==k) return v;
      return String(src).replace(/[-_]/g,' ').replace(/\b\w/g,c=>c.toUpperCase());
    }
    // Translate a raw dataset tag slug (e.g. `chungsen`, `foreclosure`,
    // `二手樓價指數`) as a friendly chip label. Falls back to the source
    // label (so a `midland` tag renders as "Midland"/"美聯物業"), then to
    // a humanized form, then to the raw tag (preserving CJK tags verbatim,
    // which are already human-readable and need no translation).
    function tagLabel(tag){
      if(!tag) return '';
      // 1. explicit tag_<slug> translation
      const k='tag_'+String(tag).toLowerCase();
      const v=t(k);
      if(v!==k) return v;
      // 2. reuse the src_<slug> translation when the tag IS a source name
      //    (chungsen / hkp / midland / aaproperty / hkma / ...)
      const sv=t('src_'+String(tag).toLowerCase());
      if(sv!=='src_'+String(tag).toLowerCase()) return sv;
      // 3. CJK / non-ASCII tags are already human-readable — return as-is
      if(/[^\x00-\x7F]/.test(tag)) return tag;
      // 4. humanize ascii slugs (e.g. `bank-owned` → `Bank Owned`)
      return String(tag).replace(/[-_]/g,' ').replace(/\b\w/g,c=>c.toUpperCase());
    }
    // Translate a cadence value (daily/weekly/monthly/...); falls back to raw.
    function cadenceLabel(cad){ if(!cad) return '—'; const k='cad_'+cad.toLowerCase(); const v=t(k); return v===k?cad:v; }
    // Render a raw record field key (e.g. `hibor_overnight`) as a human label.
    // HKMA/data.gov.hk field keys are machine names with no description column
    // from the API, so the timeline field dropdown showed raw snake_case tags.
    // This translates known fields via the `field_*` i18n namespace, then falls
    // back to a humanized form: underscores → spaces, capitalized words, and
    // common acronyms (hibor, hkma, …) expanded/cased. Pass the raw key.
    const FIELD_ACRONYMS={ hibor:'HIBOR', hkma:'HKMA', hkd:'HKD', usd:'USD', gbp:'GBP', rmb:'RMB', cny:'CNY', lb:'LB', ais:'AIS', rlb:'RLB', cds:'CDS', cpi:'CPI', gdp:'GDP', m0:'M0', m1:'M1', m2:'M2', m3:'M3' };
    function prettyField(key){
      if(!key) return '';
      // Prefer an explicit translation if one exists (covers zh-HK too).
      const k='field_'+key.toLowerCase();
      const tr=t(k);
      if(tr!==k) return tr;
      // Fall back: humanize snake_case, preserving recognized acronyms.
      return key.split('_').filter(Boolean).map(w=>{
        const up=w.toLowerCase();
        if(FIELD_ACRONYMS[up]) return FIELD_ACRONYMS[up];
        return w.charAt(0).toUpperCase()+w.slice(1);
      }).join(' ');
    }
    function valNum(v){ const n=typeof v==='number'?v:parseFloat(v); return isFinite(n)?n:null; }


