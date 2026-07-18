    // ============ routing ============
    let lastTab='overview';
    function go(tab){ lastTab=tab; document.querySelectorAll('.page').forEach(p=>p.classList.remove('active')); document.getElementById('page-'+tab).classList.add('active'); document.querySelectorAll('nav.tabs button').forEach(b=>b.classList.toggle('active',b.dataset.tab===tab)); if(location.hash!=='#'+tab) history.replaceState(null,'','#'+tab); if(tab==='datasets'){loadCategories();loadSources();} if(tab==='divergence')dvInit(); if(tab==='signals'){sigSourceFill();loadSignals();} if(tab==='cases')loadCases(); if(tab==='health')loadHealth(); if(tab==='licences'){renderLicences();loadMarketPlayers().then(renderLicences);} if(tab==='funding')renderFund(); window.scrollTo(0,0); }

    // ============ read/unread + since-you-left (P-104) ============
    function getSeen(){ try { return JSON.parse(localStorage.getItem(LS_SEEN)||'{}'); } catch(e){ return {}; } }
    function isRead(id){ return id in getSeen(); }
    function markRead(id){ const s=getSeen(); s[id]=Date.now(); try{localStorage.setItem(LS_SEEN,JSON.stringify(s));}catch(e){} }
    function getLastVisit(){ try { const v=localStorage.getItem(LS_LAST); return v?parseInt(v):null; } catch(e){ return null; } }
    function stampVisit(){ try { localStorage.setItem(LS_LAST,String(Date.now())); } catch(e){} document.title='HK City Pulse'; document.getElementById('newPipWrap')&&(document.getElementById('newPipWrap').textContent=''); }

    // ============ first-run onboarding (PR-010) ============
    // Show the orientation banner once, until the visitor dismisses it. A cold
    // visitor otherwise lands in jargon with no hand-hold (Priya D-2).
    function showOnboardIfFirstRun(){ let dismissed=false; try { dismissed=localStorage.getItem(LS_ONBOARD)==='1'; } catch(e){} const el=document.getElementById('onboardBanner'); if(el && !dismissed){ el.style.display='flex'; // translate the icon-only close button's aria-label (PR-006 rule: aria-labels are user-visible).
      const closeBtn=el.querySelector('.onboard-close'); if(closeBtn) closeBtn.setAttribute('aria-label', t('onboard_dismiss')); } }
    function dismissOnboard(){ const el=document.getElementById('onboardBanner'); if(el){ el.style.display='none'; } try { localStorage.setItem(LS_ONBOARD,'1'); } catch(e){} }

    // ============ agent presence strip (the agent works in public) ============
    let agentLastCount=0, agentLastScan=null;
    function updateAgentStrip(insightCount, health, since){
      const msg=document.getElementById('agentMsg'), when=document.getElementById('agentWhen');
      const failing=(health||[]).filter(s=>s.circuit&&s.circuit!=='closed');
      if(failing.length){ msg.innerHTML=t('agent_failing_open',{src:escapeHtml(failing.map(f=>f.source).join(', '))}); }
      else { const n=insightCount||0; const delta=n-agentLastCount; if(agentLastCount===0){ msg.innerHTML=t('agent_watching',{n}); } else if(delta>0){ msg.innerHTML=t('agent_held_new',{n,delta}); flashTitle(delta); } else { msg.innerHTML=t('agent_held_healthy',{n}); } agentLastCount=n; }
      when.textContent='checked '+relTime(new Date().toISOString());
    }
    function flashTitle(n){ if(!document.title.startsWith('(')){ document.title='('+n+') HK City Pulse'; } }
    function showOnlyNew(){ const lv=getLastVisit(); if(!lv) return; const sinceIso=new Date(lv).toISOString(); getJSON('/v1/insights?since='+encodeURIComponent(sinceIso)+'&limit=100').then(r=>{ if(!r||r.__error){alert('no new-since tracking available in this session');return;} allInsights=r; sevFilter='all'; renderInsights(); }); }

    // ============ since-you-left banner (P-104) ============
    async function renderReturnBanner(){
      const lv=getLastVisit(); const el=document.getElementById('returnBanner');
      if(!lv){ el.innerHTML=''; return; }
      // Only show once per visit; cleared after first render.
      const sinceIso=new Date(lv).toISOString();
      const fresh=await getJSON('/v1/insights?since='+encodeURIComponent(sinceIso)+'&limit=100');
      if(!fresh||fresh.__error){ el.innerHTML=''; return; }
      const n=fresh.length;
      if(n===0){ el.innerHTML=''; return; }
      const evolved=fresh.filter(i=>i.evolution&&!isRead(i.id)).length;
      el.innerHTML=`<div class="return-banner"><i class="ri-sparkling-line" style="color:var(--accent)"></i> ${t('return_new_since',{n,when:escapeHtml(relTime(sinceIso))})}${evolved?t('return_evolved',{n:evolved}):''} <button data-action="show-only-new">${t('return_show_new')}</button> <button class="dismiss" data-action="dismiss-parent" aria-label="${t('return_dismiss')}"><i class="ri-close-line"></i></button></div>`;
    }

    // ============ Proxy Divergence Radar (P-113 flagship) ============
    // Client-side mirror of detect_proxy_divergence (analysis.rs): join two
    // series on a field, then surface (1) latest-period value divergence and
    // (2) historical decoupling (Pearson r). Pure JS over fetched records —
    // keeps it zero-new-API-route and matches the detector's two sub-findings.
    // Threshold MIN_SAMPLES=4 matches the detector (not 12 — Phase 5 D-1 fix).
    let dvInited=false;
    async function dvInit(){
      if(dvInited) return; dvInited=true;
      // populate source dropdowns with all sources
      const srcs=await getJSON('/v1/sources');
      if(!srcs||srcs.__error) return;
      const seen={};
      for(const sid of ['1','2']){ const sel=document.getElementById('dvSrc'+sid); sel.innerHTML=''; for(const s of srcs){ if(!seen[s.source+sid]){ seen[s.source+sid]=true; const o=document.createElement('option'); o.value=s.source; o.textContent=s.source; sel.appendChild(o); } } }
      document.getElementById('dvSrc1').value='hkma';
      document.getElementById('dvSrc2').value='hkma';
      await dvFillDatasets('1'); await dvFillDatasets('2');
    }
    async function dvFillDatasets(sid){
      const src=document.getElementById('dvSrc'+sid).value;
      const sel=document.getElementById('dvDs'+sid); sel.innerHTML='';
      const srcs=await getJSON('/v1/sources?source='+encodeURIComponent(src));
      if(!srcs||srcs.__error) return;
      for(const s of srcs){ const o=document.createElement('option'); o.value=s.dataset; o.textContent=s.dataset+' ('+s.record_count+')'; sel.appendChild(o); }
      // sensible defaults
      if(sid==='1' && [...sel.options].some(o=>o.value==='daily-figures-interbank-liquidity')) sel.value='daily-figures-interbank-liquidity';
      if(sid==='2' && [...sel.options].some(o=>o.value==='hk-interbank-ir-daily')) sel.value='hk-interbank-ir-daily';
      await dvFillFields(sid);
    }
    async function dvFillFields(sid){
      const src=document.getElementById('dvSrc'+sid).value;
      const ds=document.getElementById('dvDs'+sid).value;
      if(!ds) return;
      // when source changes, repopulate datasets
      if(![...document.getElementById('dvDs'+sid).options].some(o=>o.value===ds)){ await dvFillDatasets(sid); return; }
      const fld=document.getElementById('dvFld'+sid); fld.innerHTML='';
      const recs=await getJSON('/v1/datasets/'+encodeURIComponent(src)+'/'+encodeURIComponent(ds)+'/records?limit=20');
      if(recs&&!recs.__error&&recs.records&&recs.records[0]){ const fields=Object.keys(recs.records[0].fields||{}).sort(); for(const f of fields){ const o=document.createElement('option'); o.value=f; o.textContent=f; fld.appendChild(o); } }
      if([...fld.options].some(o=>o.value==='hibor_overnight')) fld.value='hibor_overnight';
    }
    function dvPreset(kind){
      if(kind==='clear'){ document.getElementById('dvThreshold').value='5'; return; }
      if(kind==='hibor'){ document.getElementById('dvSrc1').value='hkma'; dvFillDatasets('1').then(()=>{ document.getElementById('dvDs1').value='daily-figures-interbank-liquidity'; dvFillFields('1'); document.getElementById('dvFld1').value='hibor_overnight'; }); document.getElementById('dvSrc2').value='hkma'; dvFillDatasets('2').then(()=>{ document.getElementById('dvDs2').value='hk-interbank-ir-daily'; dvFillFields('2'); }); document.getElementById('dvJoin').value='record_id'; document.getElementById('dvThreshold').value='5'; }
      if(kind==='land'){ document.getElementById('dvCoverage').textContent='land-premium pair needs data.gov.hk expansion — preset staged; pick two ingested fiscal series for now'; }
    }
    async function runDivergence(){
      const s1=document.getElementById('dvSrc1').value, d1=document.getElementById('dvDs1').value, f1=document.getElementById('dvFld1').value;
      const s2=document.getElementById('dvSrc2').value, d2=document.getElementById('dvDs2').value, f2=document.getElementById('dvFld2').value;
      const join=document.getElementById('dvJoin').value||'record_id';
      const thr=parseFloat(document.getElementById('dvThreshold').value)||5;
      const host=document.getElementById('dvChart'), find=document.getElementById('dvFindings');
      host.innerHTML='<div class="empty"><i class="ri-loader-4-line"></i> fetching both series…</div>'; find.innerHTML='';
      const [r1,r2]=await Promise.all([ getJSON('/v1/datasets/'+encodeURIComponent(s1)+'/'+encodeURIComponent(d1)+'/records?limit=500'), getJSON('/v1/datasets/'+encodeURIComponent(s2)+'/'+encodeURIComponent(d2)+'/records?limit=500') ]);
      if(!r1||r1.__error||!r1.records||!r2||r2.__error||!r2.records){ host.innerHTML='<div class="empty">could not fetch one or both series</div>'; return; }
      // build {key -> value} maps by join field
      const jv=(r)=>{ if(join==='record_id') return r.record_id; const v=r.fields&&r.fields[join]; return v==null?null:String(v); };
      const nv=(r,f)=>{ const v=r.fields&&r.fields[f]; if(typeof v==='number') return v; const n=parseFloat(v); return isFinite(n)?n:null; };
      const m2=new Map(); for(const r of r2.records){ const k=jv(r); const v=nv(r,f2); if(k!=null&&v!=null) m2.set(k,v); }
      // paired observations sorted by key
      let pairs=[]; for(const r of r1.records){ const k=jv(r); const a=nv(r,f1); const b=k!=null?m2.get(k):null; if(k!=null&&a!=null&&b!=null) pairs.push([k,a,b]); }
      pairs.sort((a,b)=>a[0]<b[0]?-1:1);
      if(pairs.length<4){ host.innerHTML='<div class="empty">'+t('div_no_pairs',{n:pairs.length})+'</div>'; return; }
      // (1) latest-period value divergence
      const [lk,la,lb]=pairs[pairs.length-1]; const base=Math.max(Math.abs(la),Math.abs(lb)); const deltaObs=base>1e-9?(Math.abs(la-lb)/base)*100:0;
      // (2) Pearson r over joined history
      const mean=arr=>arr.reduce((x,y)=>x+y,0)/arr.length;
      const ax=pairs.map(p=>p[1]), bx=pairs.map(p=>p[2]); const ma=mean(ax), mb=mean(bx);
      let num=0,da=0,db=0; for(let i=0;i<pairs.length;i++){ num+=(ax[i]-ma)*(bx[i]-mb); da+=Math.pow(ax[i]-ma,2); db+=Math.pow(bx[i]-mb,2); }
      const r=da>1e-9&&db>1e-9?num/Math.sqrt(da*db):0;
      // render dual timeline (reuse uPlot)
      const ts=pairs.map(p=>{ const k=p[0]; return Date.parse(k.length===7?k+'-15':k)/1000; });
      const gap=pairs.map(p=>Math.abs(p[1]-p[2]));
      host.innerHTML=''; try { const opts={ width:host.clientWidth||800, height:240, series:[{},{label:f1,stroke:'#58a6ff',width:2,points:{show:false}},{label:f2,stroke:'#d29922',width:2,points:{show:false}},{label:'|gap|',stroke:'#f85149',width:1.5,fill:'rgba(248,81,73,0.12)',points:{show:false}}],axes:[{grid:{stroke:'#30363d'},stroke:'#8b949e'},{grid:{stroke:'#30363d'},stroke:'#8b949e'}]}; new uPlot(opts,[ts,pairs.map(p=>p[1]),pairs.map(p=>p[2]),gap],host); } catch(e){ host.innerHTML='<div class="empty">'+t('tl_chart_unavail')+' ('+escapeHtml(e.message)+')</div>'; }
      // findings (two kinds, matching the detector)
      let html='<h3 style="margin:18px 0 8px">'+t('div_findings')+'</h3>';
      if(deltaObs>=thr){ const conf=Math.min((deltaObs/thr)/4,1).toFixed(2); html+=`<div class="card sev-warning"><div class="card-head"><i class="ri-error-warning-fill" style="color:var(--warn)"></i><span class="badge badge-warn">${t('div_verdict_divergence')}</span><span class="title">${escapeHtml(f1)} vs ${escapeHtml(f2)}: ${deltaObs.toFixed(1)}% apart at ${escapeHtml(lk)}</span></div><div class="summary">${t('div_proxies_disagree',{pct:deltaObs.toFixed(1),date:escapeHtml(lk),a:la.toFixed(2),b:lb.toFixed(2),thr})} <b>${escapeHtml(f1)}</b> (${escapeHtml(s1)}/${escapeHtml(d1)}) vs <b>${escapeHtml(f2)}</b> (${escapeHtml(s2)}/${escapeHtml(d2)}).</div><div class="ev-grid"><div class="ev-compare"><div class="ev-cell"><span class="ev-role">${t('lbl_primary_source')}</span><span class="ev-val">${la.toFixed(2)}</span><span class="ev-date">${escapeHtml(lk)}</span></div><div class="ev-cell"><span class="ev-role">${t('lbl_companion_source')}</span><span class="ev-val">${lb.toFixed(2)}</span><span class="ev-date">${escapeHtml(lk)}</span></div><div style="grid-column:1/-1"><span class="ev-delta" style="color:var(--warn)">gap ${deltaObs.toFixed(1)}% · conf ${conf}</span></div></div></div></div>`; }
      if(Math.abs(r)<0.6){ html+=`<div class="card sev-warning"><div class="card-head"><i class="ri-alert-line" style="color:var(--warn)"></i><span class="badge badge-warn">${t('div_verdict_decoupling')}</span><span class="title">${escapeHtml(f1)} and ${escapeHtml(f2)} have stopped moving together (${r.toFixed(2)} over ${pairs.length} periods)</span></div><div class="summary">${t('div_moved_together',{r:r.toFixed(2),n:pairs.length})} ${t('div_decoupling_followup')}</div><div class="meta"><span>${t('div_correlation_note',{r:r.toFixed(2)})}</span></div></div>`; }
      if(deltaObs<thr&&Math.abs(r)>=0.6){ html+='<div class="card sev-info"><div class="card-head"><i class="ri-information-line" style="color:var(--info)"></i><span class="badge badge-info">'+t('div_verdict_none')+'</span><span class="title">These proxies agree</span></div><div class="summary">'+t('div_proxies_agree',{pct:deltaObs.toFixed(2),thr})+' '+t('div_correlation_note',{r:r.toFixed(2)})+'</div></div>'; }
      html+='<div style="font-size:11px;color:var(--muted);margin-top:10px"><i class="ri-information-line"></i> '+t('div_methodology_footer',{n:pairs.length,join:escapeHtml(join),thr})+'</div>';
      find.innerHTML=html;
    }

    // ============ cross-source timeline (THE moat) ============
    let tlData=null, tlChart=null;
    async function loadTimeline(){
      const sel=document.getElementById('tlDataset');
      if(!sel.options.length){
        const srcs=await getJSON('/v1/sources?source=hkma');
        if(srcs&&!srcs.__error){ for(const s of srcs){ const o=document.createElement('option'); o.value=s.dataset; o.textContent=s.dataset+' ('+s.record_count+')'; if(s.dataset==='daily-interbank-liquidity')o.selected=true; sel.appendChild(o); } }
      }
      const dataset=sel.value; if(!dataset) return;
      const recs=await getJSON('/v1/datasets/hkma/'+encodeURIComponent(dataset)+'/records?limit=500');
      const press=await getJSON('/v1/datasets/press/hkma-press-releases/records?limit=500');
      if(!recs||recs.__error||!recs.records){ tlData=null; renderTimeline(); return; }
      // collect fields
      const fieldSet={};
      for(const r of recs.records){ for(const k in (r.fields||{})){ const v=valNum(r.fields[k]); if(v!==null) fieldSet[k]=(fieldSet[k]||0)+1; } }
      const fsel=document.getElementById('tlField');
      const cur=fsel.value;
      fsel.innerHTML='';
      for(const f of Object.keys(fieldSet).sort()){ const o=document.createElement('option'); o.value=f; o.textContent=f; fsel.appendChild(o); }
      if(cur&&fieldSet[cur]) fsel.value=cur; else if(fieldSet['hibor_overnight']) fsel.value='hibor_overnight';
      tlData={ records: recs.records, press: (press&&!press.__error&&press.records)?press.records:[] };
      renderTimeline();
    }
    function renderTimeline(){
      const host=document.getElementById('tlChart'); host.innerHTML='';
      if(!tlData){ host.innerHTML='<div class="tl-empty">'+t('tl_no_data')+'</div>'; document.getElementById('tlGaps').innerHTML=''; return; }
      const field=document.getElementById('tlField').value; const showPress=document.getElementById('tlShowPress').checked;
      if(!field){ host.innerHTML='<div class="tl-empty">'+t('tl_pick_field')+'</div>'; return; }
      // data points
      const pts=tlData.records.map(r=>({ date:r.record_id, v:valNum(r.fields&&r.fields[field]) })).filter(p=>p.v!==null&&p.date);
      pts.sort((a,b)=>a.date<b.date?-1:1);
      if(pts.length<2){ host.innerHTML='<div class="tl-empty">'+t('tl_not_enough')+'</div>'; document.getElementById('tlGaps').innerHTML=''; return; }
      const ts=pts.map(p=>Date.parse(p.date.length===7?p.date+'-15':p.date)/1000);
      const vals=pts.map(p=>p.v);
      const pressDates=new Set((tlData.press||[]).map(p=>p.record_id));
      // gaps: unattributed big moves (no same-day press) + press with no data
      const dataDates=new Set(pts.map(p=>p.date));
      const gaps=[];
      // big moves with no press
      const sortedVals=vals.slice().sort((a,b)=>a-b); const median=sortedVals[Math.floor(sortedVals.length/2)];
      for(let i=1;i<pts.length;i++){ const pct=Math.abs(vals[i]-vals[i-1])/Math.max(Math.abs(vals[i-1]),1e-9); if(pct>0.15){ const d=pts[i].date; const iso=d.length===7?d+'-15':d; const hasPress=[...pressDates].some(pd=>pd.startsWith(d.slice(0,7))); if(!hasPress) gaps.push({date:d, kindKey:'kind_unattributed', cls:'jump-gap', val:vals[i], pct:pct}); } }
      // press releases with no matching data (sample by month)
      const dataMonths=new Set([...dataDates].map(d=>d.slice(0,7)));
      for(const pd of pressDates){ if(pd&&pd.length>=7){ const m=pd.slice(0,7); if(!dataMonths.has(m)) gaps.push({date:pd, kindKey:'kind_press_no_data', cls:'press-gap'}); } }
      // plot with uPlot (lightweight, no build)
      const data=[ts, vals];
      const pressMarks=ts.map((t,i)=>{ const d=pts[i].date; const has=[...pressDates].some(pd=>pd.startsWith(d.slice(0,7))); return has?vals[i]:null; });
      try {
        if(tlChart){ tlChart.destroy&&tlChart.destroy(); tlChart=null; }
        const opts={ width:host.clientWidth||800, height:220, series:[
          {},
          { label:field, stroke:'var(--data)'.includes('var')?'#58a6ff':'#58a6ff', width:2, points:{show:false}, spanGaps:true },
          { label:'press release', paths:()=>null, points:{show:true, size:6, fill:'#f778ba'}, spanGaps:true }
        ], axes:[ {grid:{stroke:'#30363d'},stroke:'#8b949e'}, {grid:{stroke:'#30363d'},stroke:'#8b949e'} ], scales:{ x:{time:true}, y:{auto:true} }, cursor:{x:false,y:false} };
        tlChart=new uPlot(opts, [ts, vals, pressMarks], host);
      } catch(e){ host.innerHTML='<div class="tl-empty">'+t('tl_chart_unavail')+' ('+escapeHtml(e.message)+')</div>'; }
      // gap list
      const gl=document.getElementById('tlGaps');
      gl.innerHTML = gaps.length? '<div style="font-size:11px;color:var(--muted);margin-top:12px;margin-bottom:6px;">'+t('tl_gaps_header')+' ('+gaps.length+'):</div>' : '';
      gl.innerHTML += gaps.slice(0,8).map(g=>{ const kindStr=t(g.kindKey); return `<div class="tl-gap-row ${g.cls}" data-action="explain-and-ask" data-kind="${escapeHtml(kindStr)}" data-date="${escapeHtml(g.date)}"><span class="date">${escapeHtml(g.date)}</span><span class="desc">${escapeHtml(kindStr)}${g.val!==undefined?` · ${field} = <b>${escapeHtml(g.val.toFixed(4))}</b> (${(g.pct*100).toFixed(0)}% move)`:''}</span><span class="kind">${escapeHtml(kindStr.split(' ')[0])}</span></div>`; }).join('');
    }

    // ============ Silence Index + drill-down ============
    let silenceIdx=null;
    // Translate the server-provided Silence Index label for the active locale.
    // The server emits English ("HKMA Silence Index", "Immigration Silence
    // Index", …); this maps the institution name to zh-HK so the hero reads
    // naturally in both languages without the server needing locale awareness.
    const SILENCE_LABEL_ZH={ 'HKMA':'金管局', 'Immigration':'入境事務處', 'Land Registry':'土地註冊處', 'R&VD':'差餉物業估價處' };
    function silenceLabelZh(label){ if(!label) return t('silence_label'); let out=label; for(const en in SILENCE_LABEL_ZH){ out=out.split(en).join(SILENCE_LABEL_ZH[en]); } return out.split('Silence Index').join('沉默指數'); }
    async function loadSilence(){      const now=new Date(); const q=Math.floor(now.getUTCMonth()/3)+1; const period=now.getUTCFullYear()+'-Q'+q;
      const src=document.getElementById('silenceSource')?document.getElementById('silenceSource').value:'hkma';
      const idx=await getJSON('/v1/silence-index?source='+encodeURIComponent(src)+'&period='+period);
      // also fetch prior quarter for delta
      const prevQ=q===1?4:q-1; const prevY=q===1?now.getUTCFullYear()-1:now.getUTCFullYear(); const prevPeriod=prevY+'-Q'+prevQ;
      const prior=await getJSON('/v1/silence-index?source='+encodeURIComponent(src)+'&period='+prevPeriod);
      if(!idx||idx.__error){ document.getElementById('silenceTitle').firstChild.textContent=t('silence_unavail'); document.getElementById('silenceSub').textContent=idx&&idx.__error?('HTTP '+idx.__error):t('silence_cannot_reach'); return; }
      silenceIdx=idx;
      const score=Math.round(idx.score); const num=document.getElementById('silenceNum');
      num.textContent=score; num.style.color=score>=60?'var(--crit)':score>=30?'var(--warn)':'var(--ok)';
      document.getElementById('silenceLbl').textContent=((curLang==='zh'?silenceLabelZh(idx.label):(idx.label||t('silence_label'))))+' · '+(idx.period||period);
      document.getElementById('silenceTitle').firstChild.textContent=t('silence_events',{n:idx.total_events,s:idx.total_events===1?'':'s'});
      // delta
      if(prior&&!prior.__error&&typeof prior.score==='number'){ const d=score-Math.round(prior.score); const el=document.getElementById('silenceDelta'); if(d!==0){ // PR-009: Remix Icon arrows instead of triangle glyphs (render inconsistently across OS).
        const arrow=d>0?'<i class="ri-arrow-up-line" style="font-size:11px"></i> +':'<i class="ri-arrow-down-line" style="font-size:11px"></i> ';
        el.innerHTML=arrow+Math.abs(d)+' vs '+(prior.period||prevPeriod); el.style.color=d>0?'var(--crit)':'var(--ok)'; } }
      document.getElementById('silenceSub').textContent=t('silence_methodology',{v:idx.methodology_version});
      document.getElementById('silenceBar').style.width=score+'%';
      const sigWrap=document.getElementById('silenceSignals');
      sigWrap.innerHTML=(idx.signals||[]).filter(s=>s.count>0).map(s=>{ const dot=s.contribution>50?'var(--crit)':s.contribution>10?'var(--warn)':'var(--ok)'; const k='sk_'+s.kind; const label=t(k)===k?s.kind.replace(/_/g,' '):t(k); return `<span class="silence-signal" data-action="toggle-silence-breakdown"><span class="dot" style="background:${dot}"></span>${escapeHtml(label)}: ${s.count} (${Math.round(s.contribution)}%)</span>`; }).join('')||'<span style="color:var(--muted);font-size:12px;">'+t('silence_no_signals')+'</span>';
    }
    // P-118 v1: bind a signal that proxies "opacity spiked" via a series_jump
    // watch on the flagship HIBOR feed (the Silence Index's dominant driver).
    // The full quarterly-trend + threshold alert (v2) is gated on G2 persistence;
    // this delta v1 ships now so the watch button is functional end-to-end.
    async function watchSilenceIndex(){
      // Per-source watch target — the dominant series for each institution.
      const src=document.getElementById('silenceSource')?document.getElementById('silenceSource').value:'hkma';
      const WATCH_TARGETS={
        hkma:{ source:'hkma', dataset:'daily-figures-interbank-liquidity', field:'hibor_overnight', threshold:25.0, cadence:'daily' },
        immigration:{ source:'immigration', dataset:'daily-passenger-traffic-totals', field:'mainland_visitors', threshold:25.0, cadence:'daily' },
        landregistry:{ source:'landregistry', dataset:'monthly-transactions', field:'total_units', threshold:25.0, cadence:'monthly' },
        rvd:{ source:'rvd', dataset:'price-indices-monthly', field:'all_classes', threshold:10.0, cadence:'monthly' },
      };
      const wt=WATCH_TARGETS[src]||WATCH_TARGETS.hkma;
      const compiled={ ...wt, detector:'series_jump', comparison:'period_over_period', direction:'above' };
      const s=await postJSON('/v1/signals',{ question:'Silence Index watch — alert when a large unattributed move occurs', compiled, channels:[], owner:'dashboard' });
      const btn=document.getElementById('silenceWatchBtn');
      if(!s||s.__error){ btn.innerHTML='<i class="ri-error-warning-line"></i> '+t('silence_watch_failed',{code:(s&&s.__error)}); return; }
      btn.innerHTML='<i class="ri-check-line"></i> '+t('silence_watch_saved');
      btn.disabled=true; btn.style.opacity=0.6;
      loadSignals();
    }
    function toggleSilenceBreakdown(){
      const el=document.getElementById('silenceBreakdown');
      el.classList.toggle('open');
      if(el.classList.contains('open')&&silenceIdx){ renderSilenceBreakdown(); }
    }
    function renderSilenceBreakdown(){
      const wrap=document.getElementById('silenceRows');
      wrap.innerHTML=(silenceIdx.signals||[]).filter(s=>s.count>0).map(s=>{
        const evs=(s.evidence_ids||[]).slice(0,8).map(eid=>`<span class="ev-id" data-action="open-cite" data-id="${escapeHtml(eid)}">${escapeHtml(eid.split(':').slice(-1)[0])}<i class="ri-bookmark-line" style="margin-left:4px"></i></span>`).join('');
        const kk='sk_'+s.kind; const label=t(kk)===kk?s.kind.replace(/_/g,' '):t(kk);
        return `<div class="row" data-action="noop"><span class="ev-count">${s.count}×</span><span class="ev-kind">${escapeHtml(label)} <div class="silence-evidence">${evs||'<span style="color:var(--muted)">'+t('silence_no_ids')+'</span>'}</div></span><span class="ev-weight">${t('silence_weight')} ${s.weight}</span></div>`;
      }).join('') || '<div class="empty">'+t('silence_no_signals')+'</div>';
    }

    // ============ inline unprecedentedness band (P-103) ============
    async function loadUnprec(source, dataset, field, value){
      const u=await getJSON('/v1/unprecedentedness?source='+encodeURIComponent(source)+'&dataset='+encodeURIComponent(dataset)+'&field='+encodeURIComponent(field)+'&value='+encodeURIComponent(value));
      if(!u||u.__error||!u.band) return null;
      return u;
    }
    function unprecBandHTML(u, source, dataset, field){
      if(!u||!u.band) return '';
      const extreme=u.is_unprecedented;
      const lo=u.band.low, hi=u.band.high, min=u.hist_min??lo, max=u.hist_max??hi, val=u.value;
      const span=Math.max(max-min, Math.abs(hi-lo), 1e-9);
      const normL=((lo-min)/span)*100, normW=((hi-lo)/span)*100;
      const markerPos=Math.max(0,Math.min(100,((val-min)/span)*100));
      const pctile=u.percentile!=null?Math.round(u.percentile):null;
      const safeSrc=escapeHtml(source||''), safeDs=escapeHtml(dataset||''), safeFld=escapeHtml(field||'');
      return `<div class="unprec ${extreme?'unprec-extreme':''}">
        <div class="unprec-label"><span><i class="ri-bar-chart-2-line"></i> ${t('unprec_label')}</span>${extreme?`<span class="chip">${pctile!=null?t('unprec_top_pct',{n:Math.max(1,100-pctile)}):t('unprec_extreme')}${u.one_in_n?t('unprec_one_in_n',{n:u.one_in_n}):''}</span>`:`<span style="color:var(--ok)">${t('unprec_in_range')}</span>`}</div>
        <div class="unprec-bar">
          <div class="unprec-normal" style="left:${normL}%;width:${Math.max(2,normW)}%"></div>
          <div class="unprec-marker ${extreme?'extreme':''}" style="left:calc(${markerPos}% - 1.5px)" title="${t('unprec_current_value',{v:escapeHtml(String(val))})}"></div>
        </div>
        <div class="unprec-foot">
          <span>${t('unprec_normal_range',{lo:escapeHtml(lo.toFixed(3)),hi:escapeHtml(hi.toFixed(3))})}</span>
          ${min!==lo?`<span>${t('unprec_min',{v:escapeHtml(min.toFixed(3))})}</span>`:''}
          ${max!==hi?`<span>${t('unprec_max',{v:escapeHtml(max.toFixed(3))})}</span>`:''}
          ${u.last_exceeded?`<span>${(()=>{ const le=u.last_exceeded; const pct=(le.pct_beyond_edge>=0?'+':'')+le.pct_beyond_edge.toFixed(0); const link=`<a href="#" data-action="open-comparator" data-src="${safeSrc}" data-ds="${safeDs}" data-fld="${safeFld}" data-record="${escapeHtml(le.record_id)}" data-value="${le.value}" style="color:var(--accent);text-decoration:underline;cursor:pointer">${escapeHtml(le.record_id)} (${pct}%)</a>`; return t('unprec_last_exceeded',{link}); })()}</span>`:''}
        </div>
      </div>`;
    }
    // P-111 comparator: open the prior-period record inline as a threaded card.
    async function openComparator(source, dataset, field, recordId, value){
      // Reuse the unprecedentedness read for the prior point to show its own band,
      // and render the raw record value side-by-side with the current. Falls back
      // to a static read-only view if the record rotated.
      const u=await loadUnprec(source,dataset,field,value);
      const html=`<div class="card sev-info" style="margin-top:8px;border-left-color:var(--accent)">
        <div class="card-head"><span class="sev-icon"><i class="ri-history-line" style="color:var(--accent)"></i></span><span class="badge badge-info">${t('cmp_badge')}</span><span class="title">${escapeHtml(field)} @ ${escapeHtml(recordId)}</span></div>
        <div class="summary">${u&&u.last_exceeded?t('cmp_beyond_edge',{value,rid:escapeHtml(recordId),pct:(u.last_exceeded.pct_beyond_edge>=0?'+':'')+u.last_exceeded.pct_beyond_edge.toFixed(0)}):t('cmp_last_exceeded_plain',{value,rid:escapeHtml(recordId)})}</div>
        ${u?unprecBandHTML(u,source,dataset,field):''}
        <div class="meta"><span>${escapeHtml(source)}/${escapeHtml(dataset)}</span><span>${t('cmp_prior_exceedance')}</span></div>
      </div>`;
      // Thread it under the originating card if we can find one on-page.
      const host=document.querySelector('.card.unread .unprec, .card .unprec')||document.querySelector('#insights .card, #brief .card');
      if(host){ const wrap=document.createElement('div'); wrap.innerHTML=html; host.parentElement.insertBefore(wrap.firstElementChild, host.nextSibling); wrap.firstElementChild.scrollIntoView({behavior:'smooth',block:'center'}); }
      else { alert('Prior parallel: '+field+' @ '+recordId+' = '+value); }
    }

    // ============ human-readable side-by-side evidence ============
    function evidenceHTML(ev){
      if(!ev||!ev.length) return '';
      // group: if two entries with same field and consecutive record_ids, render side-by-side
      const byField={};
      for(const e of ev){ (byField[e.field]=byField[e.field]||[]).push(e); }
      const rows=Object.keys(byField).map(f=>{
        const arr=byField[f];
        if(arr.length>=2){
          // side-by-side compare (first = prev, last = current)
          const a=arr[0], b=arr[arr.length-1];
          const va=valNum(a.value), vb=valNum(b.value);
          let delta='';
          if(va!=null&&vb!=null){ const d=vb-va; const pct=va!==0?Math.abs(d/va*100):0; delta=`<span class="ev-delta" style="color:${d>=0?'var(--crit)':'var(--ok)'}">${d>=0?'+':''}${d.toFixed(4)} (${pct.toFixed(1)}%)</span>`; }
          return `<div class="ev-field-label">${escapeHtml(f)}</div><div class="ev-compare">
            <div class="ev-cell"><span class="ev-role">${escapeHtml(a.context||t('ev_role_previous'))}</span><span class="ev-val">${escapeHtml(JSON.stringify(a.value))}</span><span class="ev-date">${escapeHtml(a.record_id)}</span></div>
            <div class="ev-cell"><span class="ev-role">${escapeHtml(b.context||t('ev_role_current'))}</span><span class="ev-val">${escapeHtml(JSON.stringify(b.value))}</span><span class="ev-date">${escapeHtml(b.record_id)}</span></div>
            ${delta?`<div style="grid-column:1/-1">${delta}</div>`:''}
          </div>`;
        }
        return `<div class="ev-single"><span class="ev-role">${escapeHtml(arr[0].context||f)}</span> <b>${escapeHtml(JSON.stringify(arr[0].value))}</b> <span style="color:var(--muted)">@ ${escapeHtml(arr[0].record_id)}</span></div>`;
      }).join('');
      return `<details class="evidence"><summary>${t('evidence_summary',{n:ev.length})}</summary><div class="ev-grid">${rows}</div></details>`;
    }

    // ============ insight card ============
    let pendingUnprec={}; // id -> promise
    function insightCard(i, withActions){
      const exp=i.experimental?` <span class="badge badge-exp" title="${t('title_experimental')}">${t('badge_experimental')}</span>`:'';
      const evolved=i.evolution?' <span class="badge badge-evolved" title="evolved"><i class="ri-flashlight-line"></i> v'+i.version+'</span>':'';
      const unread=!isRead(i.id);
      const actions=withActions?`
        <div class="actions">
          <button data-action="open-cite" data-id="${escapeHtml(i.id)}"><i class="ri-bookmark-line"></i> ${t('cite_btn')}</button>
          <button data-action="investigate" data-id="${escapeHtml(i.id)}" data-source="${escapeHtml(i.source)}" data-dataset="${escapeHtml(i.dataset)}" data-title="${escapeHtml(i.title)}"><i class="ri-search-line"></i> ${t('investigate')}</button>
          <button data-action="load-history" data-id="${escapeHtml(i.id)}"><i class="ri-time-line"></i> ${t('history_btn')}</button>
          <button data-action="vote" data-id="${escapeHtml(i.id)}" data-useful="true"><i class="ri-thumb-up-line"></i></button>
          <button data-action="vote" data-id="${escapeHtml(i.id)}" data-useful="false"><i class="ri-thumb-down-line"></i></button>
          ${unread?`<button data-action="mark-read" data-id="${escapeHtml(i.id)}" class="active"><i class="ri-check-double-line"></i> ${t('mark_read')}</button>`:''}
          <span class="note" id="fb-${escapeHtml(i.id)}"></span>
        </div>`:'';
      return `<div class="card sev-${i.severity} ${unread?'unread':''}" id="card-${cssEsc(i.id)}" data-kind="${escapeHtml(i.kind)}">
        <div class="card-head">
          <span class="sev-icon">${SEV_ICON[i.severity]||'<i class="ri-information-line"></i>'}</span>
          <span class="badge ${SEV_BADGE[i.severity]||'badge-info'}">${escapeHtml(i.severity)}</span>${exp}${evolved}
          <span class="title">${escapeHtml(i.title)}</span>
          <span class="time" title="${escapeHtml(i.generated_at||'')}">${relTime(i.generated_at)}</span>
        </div>
        <div class="summary">${escapeHtml(i.summary)}</div>
        <div id="unprec-${cssEsc(i.id)}"></div>
        <div class="meta"><span>${escapeHtml(i.source)}/${escapeHtml(i.dataset)}</span><span>${escapeHtml(i.kind)}</span><span>${t('card_conf',{n:Math.round((i.confidence||0)*100)})}</span><span>${t('card_by',{who:escapeHtml(i.producer)})}</span></div>
        ${evidenceHTML(i.evidence)}
        <details class="history" id="hist-${cssEsc(i.id)}" style="display:none"></details>
        ${actions}
      </div>`;
    }
    // after rendering, lazy-load unprecedentedness for numeric insights.
    // THROTTLED: previously this fired one /v1/unprecedentedness call per
    // numeric insight in a tight loop — up to ~200 concurrent requests the
    // instant insights render. Each call is a compute-heavy percentile read,
    // and the fan-out OOM-killed the container on cold start, crash-looping it
    // before the in-memory store could warm (wiping all data on every restart).
    // Now processed in a small bounded concurrency window so a freshly-booted
    // container can serve these without being overwhelmed.
    async function hydrateUnprec(insights){
      const targets=[];
      for(const i of insights){
        if(i.kind!=='series_jump'&&i.kind!=='outlier'&&i.kind!=='threshold_crossing') continue;
        const ev=(i.evidence||[]).find(e=>e.context&&e.context.includes('current'))||(i.evidence||[])[0];
        if(!ev) continue;
        const v=valNum(ev.value); if(v===null) continue;
        targets.push({i, ev, v});
      }
      const CONCURRENCY=5;
      let idx=0;
      async function worker(){
        while(idx<targets.length){
          const {i,ev,v}=targets[idx++]; // claim next; workers share idx
          const u=await loadUnprec(i.source, i.dataset, ev.field, v);
          const host=document.getElementById('unprec-'+cssEsc(i.id));
          if(host&&u) host.innerHTML=unprecBandHTML(u, i.source, i.dataset, ev.field);
        }
      }
      // Spawn a bounded pool of workers; each pulls the next target until done.
      await Promise.all(Array.from({length:Math.min(CONCURRENCY,targets.length)},worker));
    }

    // ============ brief + facets ============
    function dedupDiversify(items, maxN){
      const key=it=>it.source+'/'+it.dataset+'/'+((it.evidence&&it.evidence[0]&&it.evidence[0].field)||'');
      const groups={}; for(const it of items){ (groups[key(it)]=groups[key(it)]||[]).push(it); }
      const groupKeys=Object.keys(groups); const out=[]; let added=true, idx=0; const pointers={};
      while(added&&out.length<maxN){ added=false; for(const gk of groupKeys){ const cap=idx<2?2:999; if((pointers[gk]||0)<groups[gk].length&&(pointers[gk]||0)<cap){ out.push(groups[gk][pointers[gk]||0]); pointers[gk]=(pointers[gk]||0)+1; added=true; if(out.length>=maxN)break; } } idx++; }
      return out;
    }
    let briefAll=[], briefFilters={};
    const BRIEF_PCT_RE=/moved\s*([+\-])\s*([\d.]+)\s*%/;
    function briefFacets(it){ const field=(it.evidence&&it.evidence[0]&&it.evidence[0].field)||''; let direction='',magnitude=''; const m=BRIEF_PCT_RE.exec(it.title||it.summary||''); if(m){ const pct=parseFloat(m[2]); direction=m[1]==='+'?'up':'down'; magnitude=pct>=30?'major':pct>=15?'moderate':'small'; } return {severity:it.severity||'',kind:it.kind||'',source:it.source+'/'+it.dataset,field,direction,magnitude}; }
    function buildBriefFacetOptions(items){ const facets=['severity','kind','source','field','direction','magnitude']; const out={}; for(const f of facets){ const counts={}; for(const it of items){ const v=briefFacets(it)[f]; if(!v)continue; counts[v]=(counts[v]||0)+1; } const opts=Object.keys(counts).sort().map(v=>({value:v,count:counts[v]})); if(opts.length>1) out[f]=opts; } return out; }
    // PR-006b: facet labels resolve through t() so they follow the language toggle.
    const FACET_LABEL_KEYS={severity:'facet_severity',kind:'facet_kind',source:'facet_source',field:'facet_field',direction:'facet_direction',magnitude:'facet_magnitude'};
    function facetValueLabel(facet,value){
      const map={direction:{up:'facet_up',down:'facet_down'},magnitude:{major:'facet_major',moderate:'facet_moderate',small:'facet_small'},severity:{critical:'facet_critical',warning:'facet_warning',info:'facet_info'}};
      const key=(map[facet]&&map[facet][value]);
      return key?t(key):value;
    }
    function renderBriefFilters(){ const wrap=document.getElementById('briefFilters'); const options=buildBriefFacetOptions(briefAll); const facetKeys=Object.keys(options); if(!facetKeys.length){ wrap.style.display='none'; wrap.innerHTML=''; return; } wrap.style.display='flex'; let html=''; for(const f of facetKeys){ html+=`<span style="color:var(--muted);font-size:11px;text-transform:uppercase;align-self:center;margin-right:2px;">${escapeHtml(t(FACET_LABEL_KEYS[f]||f))}:</span>`; for(const o of options[f]){ const active=briefFilters[f]===o.value; const lbl=facetValueLabel(f,o.value); html+=`<button class="${active?'active':''}" data-action="toggle-brief-filter" data-filter="${f}" data-value="${escapeHtml(o.value)}">${escapeHtml(lbl)} <span style="opacity:0.7">${o.count}</span></button>`; } } if(Object.keys(briefFilters).length){ html+=`<span class="filter-clear" data-action="clear-brief-filters" role="button">${t('facet_clear')} <i class="ri-close-line"></i></span>`; } wrap.innerHTML=html; }
    function toggleBriefFilter(facet,value){ if(briefFilters[facet]===value) delete briefFilters[facet]; else briefFilters[facet]=value; renderBriefFilters(); renderBrief(); }
    function clearBriefFilters(){ briefFilters={}; renderBriefFilters(); renderBrief(); }
    function renderBrief(){ const el=document.getElementById('brief'); let shown=briefAll; const active=Object.keys(briefFilters); if(active.length){ shown=briefAll.filter(it=>{ const f=briefFacets(it); return active.every(k=>f[k]===briefFilters[k]); }); } const top=shown.slice(0,8); if(!top.length){ el.innerHTML='<div class="empty">no findings match these filters</div>'; return; } el.innerHTML=top.map(it=>insightCard(it,true)).join(''); hydrateUnprec(top); }
    async function loadBrief(){ const b=await getJSON('/v1/brief?limit=50'+langParam()); const el=document.getElementById('brief'); if(!b||b.__error||!b.items||!b.items.length){ el.innerHTML=await degradedEmpty(curLang==='zh'?'尚無簡報。':'No brief yet.'); document.getElementById('briefCount').textContent=''; document.getElementById('briefFilters').style.display='none'; return; } briefAll=dedupDiversify(b.items,20); briefFilters={}; document.getElementById('briefCount').textContent=`(${briefAll.length})`; renderBriefFilters(); renderBrief(); }

    // ============ insights feed ============
    let allInsights=[], sevFilter='all';
    function setSevFilter(s){ sevFilter=s; document.querySelectorAll('.insights-filter button[data-sev]').forEach(b=>b.classList.toggle('active',b.dataset.sev===s)); renderInsights(); }
    function renderInsights(){ const el=document.getElementById('insights'); const filtered=sevFilter==='all'?allInsights:allInsights.filter(i=>i.severity===sevFilter); if(!filtered.length){ el.innerHTML='<div class="empty">'+t('no_insights_sev')+'</div>'; return; } const top=filtered.slice(0,60); el.innerHTML=top.map(i=>insightCard(i,true)).join(''); hydrateUnprec(top); anchorFromHash(); }
    async function loadInsights(){ const ins=await getJSON('/v1/insights?limit=100'+langParam()); if(!ins||ins.__error) return; allInsights=ins; renderInsights(); updateAgentStrip(ins.length, cachedHealth); }

    // ============ degraded-state honesty ============
    let cachedHealth=null;
    async function loadHealthQuiet(){ const hs=await getJSON('/v1/health/sources'); if(hs&&!hs.__error){ cachedHealth=hs; return hs; } const alt=await getJSON('/health/sources'); if(alt&&!alt.__error){ cachedHealth=alt; return alt; } return []; }
    async function degradedEmpty(message){ const hs=cachedHealth||await loadHealthQuiet(); const failing=(hs||[]).filter(s=>s.circuit&&s.circuit!=='closed'); if(failing.length){ return `<div class="empty">${escapeHtml(message)} <span style="color:var(--warn)">— ${t('degraded_upstream',{srcs:failing.map(f=>escapeHtml(f.source)).join(', ')})} <i class="ri-refresh-line"></i> ${t('degraded_retry')}</span></div>`; } // If health is empty the API itself is unreachable (e.g. the public static
    // deploy with no API origin wired). Say so honestly rather than claiming the
    // agent "scans periodically" — that is only true when the API is reachable.
    if(!hs||!hs.length){ return `<div class="empty">${escapeHtml(message)} ${t('degraded_unreachable')}</div>`; }
    return `<div class="empty">${escapeHtml(message)} ${t('degraded_scans')}</div>`; }
    async function renderDegradedBanner(){ const hs=cachedHealth||await loadHealthQuiet(); const el=document.getElementById('degradedBanner'); const failing=(hs||[]).filter(s=>s.circuit&&s.circuit!=='closed'); el.innerHTML=failing.length?`<div class="banner warn"><i class="ri-alert-line"></i> ${t('degraded_banner',{srcs:failing.map(f=>'<b>'+escapeHtml(f.source)+'</b> ('+escapeHtml(f.circuit)+')').join(', ')})} <button data-action="load-all">${t('degraded_retry_now')}</button></div>`:''; }

    // ============ feedback ============
    async function vote(id, useful, btn){ const {base,headers}=api(); let ok=false; try { const r=await fetch(base+'/v1/insights/'+encodeURIComponent(id)+'/feedback',{method:'POST',headers:{'Content-Type':'application/json',...headers},body:JSON.stringify({useful})}); ok=r.ok; } catch(e){ ok=false; } const note=document.getElementById('fb-'+id); if(note) note.textContent=ok?(useful?'thanks — marked useful':'thanks — marked not useful'):'failed (HTTP) — not recorded'; }

    // ============ Cite-It modal + permalink ============
    let citeId=null, citeFmt='bibtex', citeManifestObj=null, citeBundleObj=null;
    // The permalink must point at the real public origin (window.location.origin)
    // so a shared link actually resolves for whoever opens it — NOT localhost:8080,
    // which is only the API's fallback when no base_url is supplied. PR-002.
    function citeBaseParam(){ return 'base_url='+encodeURIComponent(window.location.origin); }
    async function openCite(id){ citeId=id; document.getElementById('citeModal').classList.add('open'); // permalink first
      const bundle=await getJSON('/v1/insights/'+encodeURIComponent(id)+'/cite?'+citeBaseParam()); citeBundleObj=(bundle&&!bundle.__error)?bundle:null; document.getElementById('citePermalink').textContent=citeBundleObj?citeBundleObj.permalink:('…/cite/'+id); setFmt('bibtex'); const m=citeBundleObj&&citeBundleObj.manifest; citeManifestObj=m; document.getElementById('citeManifest').innerHTML=m?`Reproducibility manifest · detector <code>${escapeHtml(m.detector)}</code> · data SHA-256 <code>${escapeHtml(m.data_sha256).slice(0,16)}…</code>${citeBundleObj.experimental?' · <i class="ri-alert-line" style="color:var(--crit)"></i> experimental':''}`:''; }
    function closeCite(){ document.getElementById('citeModal').classList.remove('open'); }
    async function setFmt(fmt){ citeFmt=fmt; document.querySelectorAll('#fmtTabs button').forEach(b=>b.classList.toggle('active',b.dataset.fmt===fmt)); document.getElementById('citeOut').textContent='loading…'; const txt=await fetchText('/v1/insights/'+encodeURIComponent(citeId)+'/cite?format='+fmt+'&'+citeBaseParam()); document.getElementById('citeOut').textContent=txt||'(error loading)'; }
    async function copyCite(){ try { await navigator.clipboard.writeText(document.getElementById('citeOut').textContent); } catch(e){ const ta=document.createElement('textarea'); ta.value=document.getElementById('citeOut').textContent; document.body.appendChild(ta); ta.select(); document.execCommand('copy'); ta.remove(); } }
    async function copyPermalink(){ const url=document.getElementById('citePermalink').textContent; try { await navigator.clipboard.writeText(url); } catch(e){} }
    async function citeBundle(){ if(!citeBundleObj) return; const blob=new Blob([JSON.stringify(citeBundleObj,null,2)],{type:'application/json'}); const a=document.createElement('a'); a.href=URL.createObjectURL(blob); a.download='cite-'+cssEsc(citeId).slice(0,40)+'.json'; a.click(); }

    // ============ insight history ============
    async function loadHistory(id, btn){ const det=document.getElementById('hist-'+cssEsc(id)); const open=det.style.display==='none'; det.style.display=open?'block':'none'; if(!open) return; det.innerHTML='<summary style="color:var(--accent);font-size:12px">loading…</summary>'; const h=await getJSON('/v1/insights/'+encodeURIComponent(id)+'/history'); if(!h||h.__error){ det.innerHTML='<span style="color:var(--muted);font-size:12px">error</span>'; return; } if(!h.length){ det.innerHTML='<div style="color:var(--muted);font-size:12px;margin-top:6px">No prior versions — insights regenerate each scan pass.</div>'; return; } det.innerHTML='<summary style="cursor:pointer;color:var(--accent);font-size:12px">history ('+h.length+')</summary>'+h.map(v=>`<div style="font-size:12px;color:var(--muted);margin-top:6px">${escapeHtml(v.generated_at||'')} · ${escapeHtml(v.version?'v'+v.version:'')} · ${escapeHtml(v.snapshot.severity)} · ${escapeHtml(v.snapshot.summary||v.snapshot.title||'')}</div>`).join(''); }

    // ============ investigations (real workspace) ============
    let activeInvId=null;
    async function investigate(id, source, dataset, title){
      const inv=await postJSON('/v1/investigations',{seed_insight_id:id,seed_source:source,seed_dataset:dataset,seed_title:title});
      if(!inv||inv.__error){ alert('Could not create case: '+(inv&&inv.__text?inv.__text:'HTTP '+(inv&&inv.__error))); return; }
      activeInvId=inv.id; go('cases'); openInvWorkspace(inv.id);
    }
    async function loadCases(){ const list=await getJSON('/v1/investigations?limit=50'); const el=document.getElementById('casesList'); if(list&&list.__error===401){ el.innerHTML='<div class="empty">'+(t('auth_needed_cases'))+'</div>'; document.getElementById('navCases').textContent='0'; return; } if(!list||list.__error||!list.length){ el.innerHTML='<div class="empty">'+t('cases_none')+'</div>'; document.getElementById('navCases').textContent='0'; return; } document.getElementById('navCases').textContent=list.length; el.innerHTML=list.map(c=>`<div class="case-card" data-action="open-inv-workspace" data-id="${escapeHtml(c.id)}"><div class="ct">${escapeHtml(c.title||'(untitled)')} <i class="ri-arrow-right-line" style="color:var(--accent)"></i></div><div class="cm">seed: ${escapeHtml(c.seed_source||'?')}/${escapeHtml(c.seed_dataset||'?')} · ${escapeHtml(c.status||(c.steps&&c.steps.length?c.steps.length+' steps':'empty'))} · ${escapeHtml(relTime(c.updated_at||c.created_at))}</div></div>`).join(''); }
    async function openInvWorkspace(id){
      const inv=await getJSON('/v1/investigations/'+encodeURIComponent(id));
      const mount=document.getElementById('invWorkspaceMount');
      if(!inv||inv.__error){ mount.innerHTML='<div class="empty">case not found</div>'; return; }
      activeInvId=id;
      const stepsHTML=(inv.steps||[]).map((s,i)=>`<div class="inv-step"><div class="num">${i+1}</div><div class="body"><div class="prompt">${escapeHtml(s.prompt)} <span style="color:var(--muted)">(${escapeHtml(s.kind)})</span></div>${s.answer?`<div class="answer">${escapeHtml(s.answer.text)} <span style="color:var(--muted);font-size:11px">${Math.round((s.answer.confidence||0)*100)}%</span></div>`:''}${s.trace&&s.trace.length?`<div class="trace-mini"><i class="ri-tools-line"></i> ${s.trace.length} tool call(s): ${escapeHtml(s.trace.map(t=>t.tool).join(', '))}</div>`:''}${s.annotation?`<div class="trace-mini" style="color:var(--accent)"><i class="ri-sticky-note-line"></i> ${escapeHtml(s.annotation)}</div>`:''}</div></div>`).join('');
      mount.innerHTML=`<div class="inv-workspace">
        <div class="inv-head"><span class="t">${escapeHtml(inv.title)}</span><span style="color:var(--muted);font-size:12px">${escapeHtml(relTime(inv.updated_at))}</span><button class="icon-btn" data-action="close-investigation"><i class="ri-close-line"></i> close</button></div>
        <div class="inv-seed"><b>${t('inv_seed_insight')}</b> ${escapeHtml(inv.seed_title)} <span style="color:var(--muted)">(${escapeHtml(inv.seed_source)}/${escapeHtml(inv.seed_dataset)})</span></div>
        <div class="inv-chips"><span style="color:var(--muted);align-self:center;font-size:12px">${t('inv_guided_steps')}</span><button class="chip-btn" data-action="inv-chip" data-prompt="related series"><i class="ri-line-chart-line"></i> ${t('inv_chip_related')}</button><button class="chip-btn" data-action="inv-chip" data-prompt="historical parallels"><i class="ri-history-line"></i> ${t('inv_chip_parallels')}</button><button class="chip-btn" data-action="inv-chip" data-prompt="cross-source check"><i class="ri-share-line"></i> ${t('inv_chip_cross')}</button></div>
        <div class="inv-steps">${stepsHTML||'<div class="empty">'+t('inv_no_steps')+'</div>'}</div>
        <div class="inv-input"><input id="invInput" placeholder="${t('inv_input_ph')}" data-action="inv-ask-on-enter" /><button class="icon-btn" data-action="inv-ask"><i class="ri-send-plane-line"></i></button></div>
      </div>`;
      mount.scrollIntoView({behavior:'smooth',block:'start'});
    }
    async function invChip(label){ const q='For the seed insight, show '+label+'.'; await invAskWith(q, label, 'chip'); }
    async function invAsk(){ const inp=document.getElementById('invInput'); const q=inp.value.trim(); if(!q) return; inp.value=''; await invAskWith(q, q, 'qa'); }
    async function invAskWith(prompt, label, kind){
      if(!activeInvId) return;
      // optimistic: append a thinking step
      const steps=document.querySelector('.inv-steps'); const n=steps.children.length+1;
      const think=document.createElement('div'); think.className='inv-step'; think.innerHTML=`<div class="num">${n}</div><div class="body"><div class="prompt">${escapeHtml(label)}</div><div class="answer" style="color:var(--muted)"><i class="ri-loader-4-line"></i> agent investigating…</div></div>`; steps.appendChild(think); think.scrollIntoView({behavior:'smooth'});
      const a=await postJSON('/v1/ask',{question:prompt});
      const trace=(a&&!a.__error&&a.trace)?a.trace:[];
      await postJSON('/v1/investigations/'+encodeURIComponent(activeInvId)+'/steps',{kind, prompt:label, answer:(a&&!a.__error)?{text:a.text,confidence:a.confidence}:{text:'(error)',confidence:0}, trace});
      openInvWorkspace(activeInvId);
    }

    // ============ signals ============
    async function sigSourceFill(){ const sel=document.getElementById('sigSource'); if(sel.options.length) return; const srcs=await getJSON('/v1/sources'); if(!srcs||srcs.__error) return; const seen={}; for(const s of srcs) if(!seen[s.source]){ seen[s.source]=true; const o=document.createElement('option'); o.value=s.source; o.textContent=s.source; sel.appendChild(o); } sel.value='hkma'; sigDatasetFill(); }
    async function sigDatasetFill(){ const src=document.getElementById('sigSource').value; const sel=document.getElementById('sigDataset'); const field=(document.getElementById('sigField').value||'').trim(); sel.innerHTML=''; const srcs=await getJSON('/v1/sources?source='+encodeURIComponent(src)); if(!srcs||srcs.__error) return; for(const s of srcs){ const o=document.createElement('option'); o.value=s.dataset; o.textContent=s.dataset+' ('+s.record_count+')'; sel.appendChild(o); } if(field){ const preferred=src==='hkma'&&field==='hibor_overnight'?'daily-figures-interbank-liquidity':null; let chosen=null; if(preferred&&[...sel.options].some(o=>o.value===preferred)) chosen=preferred; else { for(const s of srcs.slice(0,12)){ try { const r=await getJSON('/v1/datasets/'+encodeURIComponent(s.source)+'/'+encodeURIComponent(s.dataset)+'/records?limit=1'); if(r&&!r.__error&&r.records&&r.records[0]&&(field in (r.records[0].fields||{}))){ chosen=s.dataset; break; } } catch(e){} } } if(chosen) sel.value=chosen; } }
    function buildScanTarget(){ return { source:document.getElementById('sigSource').value, dataset:document.getElementById('sigDataset').value, detector:document.getElementById('sigDetector').value, field:document.getElementById('sigField').value||null, threshold:parseFloat(document.getElementById('sigThreshold').value)||null, comparison:'period_over_period', cadence:document.getElementById('sigCadence').value||'daily', direction:document.getElementById('sigDirection').value }; }
    async function previewSignal(){ const box=document.getElementById('sigPreview'); box.textContent=t('sig_previewing'); const compiled=buildScanTarget(); const p=await postJSON('/v1/signals/preview',{compiled, window_days:90}); if(!p||p.__error){ box.innerHTML='<span style="color:var(--crit)">'+t('sig_preview_err',{detail:escapeHtml((p&&p.__text)||('HTTP '+(p&&p.__error)))})+'</span>'; return; } const n=p.count==null?(p.findings||[]).length:p.count; box.innerHTML=t('sig_fired',{n,days:(p.window_days||90),s:(n===1?'':'s'),det:escapeHtml(p.compiled.detector),src:escapeHtml(p.compiled.source)+'/'+escapeHtml(p.compiled.dataset)})+'<br>'+(n?'<span style="color:var(--muted)">'+t('sig_recent',{list:((p.findings||[]).slice(0,3).map(f=>escapeHtml(f.record_id||f.title||'?')).join(', '))})+'</span>':'<span style="color:var(--ok)">'+t('sig_none_fired')+'</span>'); }
    async function saveSignal(){ const q=document.getElementById('sigQuestion').value.trim(); const compiled=buildScanTarget(); const s=await postJSON('/v1/signals',{question:q||null, compiled, channels:[], owner:'dashboard'}); if(!s||s.__error){ alert(t('sig_could_not_save',{detail:((s&&s.__text)||('HTTP '+(s&&s.__error)))})); return; } document.getElementById('sigQuestion').value=''; const note=document.getElementById('sigSaveNote'); if(note){ note.textContent=t('sig_saved_ok'); note.style.display='block'; setTimeout(()=>{ note.style.display='none'; }, 6000); } loadSignals(); }
    async function loadSignals(){ const list=await getJSON('/v1/signals?limit=50'); const el=document.getElementById('signalsList'); if(list&&list.__error===401){ el.innerHTML='<div class="empty">'+(t('auth_needed_signals'))+'</div>'; document.getElementById('navSignals').textContent='0'; return; } if(!list||list.__error||!list.length){ el.innerHTML='<div class="empty">'+t('sig_none')+'</div>'; document.getElementById('navSignals').textContent='0'; return; } document.getElementById('navSignals').textContent=list.length; el.innerHTML=list.map(s=>`<div class="case-card"><div class="ct">${escapeHtml(s.question||(s.compiled.detector+' on '+s.compiled.source+'/'+s.compiled.dataset))}</div><div class="cm"><code>${escapeHtml(s.compiled.detector)}</code> · ${escapeHtml(s.compiled.source)}/${escapeHtml(s.compiled.dataset)} · ${s.enabled?t('sig_enabled'):t('sig_paused')}</div><div class="actions"><button data-action="toggle-signal" data-id="${escapeHtml(s.id)}" data-enable="${!s.enabled}">${s.enabled?t('sig_pause'):t('sig_enable')}</button><button data-action="del-signal" data-id="${escapeHtml(s.id)}">${t('sig_delete')}</button><button data-action="load-dispatch-log" data-id="${escapeHtml(s.id)}"><i class="ri-history-line"></i> ${t('sig_dispatch')}</button></div><div class="dispatch-log" id="dlog-${escapeHtml(s.id)}" style="display:none;margin-top:8px;"></div></div>`).join(''); }
    // P-110: surface the shipped GET /v1/alerts as a per-signal dispatch timeline.
    // Shows whether each signal fired / delivered / bounced — the "did my signal
    // actually reach me?" trust layer that was missing.
    async function loadDispatchLog(sigId, btn){
      const host=document.getElementById('dlog-'+sigId);
      const open=host.style.display==='block';
      host.style.display=open?'none':'block';
      if(open) return;
      host.innerHTML='<div style="color:var(--muted);font-size:12px;padding:6px 0"><i class="ri-loader-4-line"></i> '+t('sig_loading_disp')+'</div>';
      const alerts=await getJSON('/v1/alerts?limit=50');
      if(!alerts||alerts.__error){ host.innerHTML='<div style="color:var(--muted);font-size:12px;padding:6px 0">'+t('sig_needs_feature')+' <span class="mode-badge">'+t('mode_heuristic')+'</span></div>'; return; }
      if(!alerts.length){ host.innerHTML='<div style="color:var(--muted);font-size:12px;padding:6px 0">'+t('sig_never_fired')+'</div>'; return; }
      // PR-Marcus: this is the GLOBAL fleet alert log, not per-signal (per-signal
      // logs arrive with the P-108 identity tier). Label it honestly, translate
      // the status literals, and drop the non-functional "resend" button (a fake
      // control is worse than none on a desk).
      function dispStatusLabel(status){ return {delivered:t('sig_status_delivered'),bounced:t('sig_status_bounced'),failed:t('sig_status_failed')}[status]||t('sig_status_unknown'); }
      host.innerHTML='<div style="font-size:11px;color:var(--muted);text-transform:uppercase;margin-bottom:4px">'+t('sig_disp_title',{n:alerts.length})+'</div><div style="font-size:11px;color:var(--muted);margin-bottom:8px">'+t('sig_disp_intro')+'</div>'+alerts.slice(0,12).map(a=>{ const status=a.status||'unknown'; const color=status==='delivered'?'var(--ok)':(status==='bounced'||status==='failed')?'var(--crit)':'var(--warn)'; return '<div style="display:flex;flex-wrap:wrap;gap:8px;font-size:12px;padding:4px 0;border-bottom:1px solid var(--border)"><span style="color:'+color+';font-weight:600">'+escapeHtml(dispStatusLabel(status))+'</span><span style="flex:1;color:#c9d1d9;min-width:120px">'+escapeHtml(a.kind||a.insight_id||t('sig_disp_fallback'))+'</span><span style="color:var(--muted)">'+escapeHtml(relTime(a.dispatched_at))+'</span></div>'; }).join('');
    }
    async function toggleSignal(id, en){ const cur=await getJSON('/v1/signals/'+encodeURIComponent(id)); if(!cur||cur.__error) return; cur.enabled=en; await fetch(api().base+'/v1/signals/'+encodeURIComponent(id),{method:'PATCH',headers:{'Content-Type':'application/json',...api().headers},body:JSON.stringify(cur)}); loadSignals(); }
    async function delSignal(id){ await fetch(api().base+'/v1/signals/'+encodeURIComponent(id),{method:'DELETE',headers:api().headers}); loadSignals(); }

    // ============ datasets page ============
    async function loadCategories(){ const cats=await getJSON('/v1/categories'); const sel=document.getElementById('catFilter'); const cur=sel.value; sel.innerHTML='<option value="">'+t('th_all_categories')+'</option>'; if(cats&&!cats.__error) for(const c of cats){ const o=document.createElement('option'); o.value=c.category; o.textContent=`${catLabel(c.category)} (${c.count})`; sel.appendChild(o); } sel.value=cur; }
    async function loadSources(){ const cat=document.getElementById('catFilter').value; const q=document.getElementById('srcSearch').value.trim(); const params=new URLSearchParams(); if(cat) params.set('category',cat); if(q) params.set('q',q); const qs=params.toString()?('?'+params.toString()):''; const src=await getJSON('/v1/sources'+qs); const sb=document.getElementById('sources'); if(src&&!src.__error&&src.length){ sb.innerHTML=src.map(s=>`<tr><td>${catBadge(s.category)}</td><td>${escapeHtml(s.source)}/${escapeHtml(s.dataset)}<br><span style="color:var(--muted)">${escapeHtml(s.title)}</span></td><td>${(s.tags||[]).map(tt=>`<span class="tag-chip" data-action="tag-search" data-tag="${escapeHtml(tt)}">${escapeHtml(tt)}</span>`).join('')||'—'}</td><td style="color:var(--muted)">${escapeHtml(cadenceLabel(s.cadence))}</td><td>${s.record_count}</td></tr>`).join(''); } else { sb.innerHTML='<tr><td class="empty" colspan="5">'+t('ds_no_match')+'</td></tr>'; } }
    function tagSearch(t){ document.getElementById('srcSearch').value=t; loadSources(); }

    // ============ health page ============
    async function loadHealth(){ const hs=await loadHealthQuiet(); const el=document.getElementById('health'); if(!hs||hs.__error||!hs.length){ el.innerHTML='<span class="empty">unavailable</span>'; return; } const srcs=await getJSON('/v1/sources'); const counts={}; if(srcs&&!srcs.__error) for(const s of srcs) counts[s.source]=(counts[s.source]||0)+(s.record_count||0); el.innerHTML=hs.map(s=>{ const closed=s.circuit==='closed'; const color=closed?'var(--ok)':'var(--crit)'; const zero=(counts[s.source]||0)===0; return `<div class="lic-hl"><div class="name">${escapeHtml(s.source)}</div><div class="who">circuit: <span style="color:${color}">${escapeHtml(s.circuit)}</span></div><div class="who">${counts[s.source]||0} records ${zero?'<span style="color:var(--crit)">(empty)</span>':''}</div></div>`; }).join(''); }

    // ============ chat rail (agent workspace lite) ============
    const chatLog=[];
    async function askAgent(){ const input=document.getElementById('askInput'); let q=input.value.trim(); if(!q) return; if(q.startsWith('/')){ q='Investigate: '+q.slice(1); } input.value=''; chatLog.push({q,loading:true}); renderChat(); const a=await postJSON('/v1/ask',{question:q}); chatLog[chatLog.length-1]={q, a:(a&&!a.__error)?a:{text:'error: '+((a&&a.__error)||'failed'),confidence:0}}; renderChat(); }
    function renderChat(){ const el=document.getElementById('chatLog'); document.getElementById('chatEmpty')?.remove(); el.innerHTML=chatLog.map(m=>{ if(m.loading) return `<div class="chat-msg"><div class="q">you: ${escapeHtml(m.q)}</div><div class="a" style="color:var(--muted)"><i class="ri-loader-4-line"></i> agent thinking…</div></div>`; const conf=m.a.confidence!==undefined?` <span class="conf">${Math.round(m.a.confidence*100)}%</span>`:''; const inlineEvidence=(m.a.trace&&m.a.trace.length)?`<div class="ev-inline"><i class="ri-tools-line"></i> ${m.a.trace.length} tool call(s): ${escapeHtml(m.a.trace.map(t=>t.tool).join(', '))}</div>`:''; const trace=(m.a.trace&&m.a.trace.length)?`<details><summary>full trace</summary><pre>${escapeHtml(JSON.stringify(m.a.trace,null,1))}</pre></details>`:''; return `<div class="chat-msg"><div class="q">you: ${escapeHtml(m.q)}</div><div class="a">${escapeHtml(m.a.text)}${conf}</div>${inlineEvidence}${trace}</div>`; }).join(''); el.scrollTop=el.scrollHeight; }
    document.getElementById('askInput').addEventListener('keydown', e=>{ if(e.key==='Enter') askAgent(); });

    // ============ deep-link anchoring ============
    function anchorFromHash(){ const m=location.hash.match(/^#card-(.+)$/); if(m){ const card=document.getElementById('card-'+m[1]); if(card){ card.classList.add('anchored'); card.scrollIntoView({behavior:'smooth',block:'center'}); } } }
    // cite share permalink: open cite modal on load if the URL targets a cite.
    // Accepts BOTH forms the system emits: a hash `#cite/<id>` and a path
    // `/cite/<id>` (the Cite-It permalink from build_citation, and the route
    // served by the API + the Netlify SPA rewrite). Without the path form a
    // shared permalink 404s / never opens the drawer.
    function checkShareLanding(){
      let id=null;
      const hm=location.hash.match(/^#cite\/(.+)$/);
      if(hm){ id=decodeURIComponent(hm[1]); }
      else { const pm=location.pathname.match(/\/cite\/(.+)$/); if(pm){ id=decodeURIComponent(pm[1]); } }
      if(id){ openCite(id); }
    }

    // ============ command palette ============
    function openPalette(){ document.getElementById('palette').classList.add('open'); document.getElementById('paletteInput').value=''; document.getElementById('paletteInput').focus(); renderPalette(); }
    function closePalette(){ document.getElementById('palette').classList.remove('open'); }
    let paletteItems=[], paletteSel=0;
    function renderPalette(){ const q=document.getElementById('paletteInput').value.trim().toLowerCase(); paletteItems=[]; // pages
      const pages=[{t:'Overview',tab:'overview',icon:'ri-dashboard-line'},{t:'Divergences',tab:'divergence',icon:'ri-radar-line'},{t:'Datasets',tab:'datasets',icon:'ri-database-2-line'},{t:'Signals',tab:'signals',icon:'ri-notification-3-line'},{t:'Cases',tab:'cases',icon:'ri-folder-line'},{t:'System health',tab:'health',icon:'ri-pulse-line'},{t:'Licences',tab:'licences',icon:'ri-government-line'},{t:'Funding & Credits',tab:'funding',icon:'ri-hand-coin-line'}];
      for(const p of pages) if(!q||p.t.toLowerCase().includes(q)) paletteItems.push({type:'page',...p});
      // insights
      for(const i of allInsights.slice(0,40)){ if(!q||(i.title+' '+i.summary+' '+i.kind).toLowerCase().includes(q)) paletteItems.push({type:'insight',t:i.title,icon:'ri-lightbulb-line',id:i.id}); }
      if(paletteSel>=paletteItems.length) paletteSel=0;
      document.getElementById('paletteList').innerHTML=paletteItems.slice(0,30).map((it,idx)=>`<div class="palette-item ${idx===paletteSel?'sel':''}" data-action="palette-pick" data-idx="${idx}"><span class="pi-icon"><i class="${it.icon}"></i></span><span class="pi-title">${escapeHtml(it.t)}</span><span class="pi-hint">${it.type}${it.id?' · enter to cite':''}</span></div>`).join('')||'<div class="empty" style="padding:24px">'+t('pal_no_match')+'</div>';
    }
    function palettePick(idx){ const it=paletteItems[idx]; if(!it) return; if(it.type==='page'){ go(it.tab); } else if(it.type==='insight'){ go('overview'); setTimeout(()=>{ const c=document.getElementById('card-'+cssEsc(it.id)); if(c){ c.classList.add('anchored'); c.scrollIntoView({behavior:'smooth',block:'center'}); } else { openCite(it.id); } },200); } closePalette(); }
    document.getElementById('paletteInput').addEventListener('keydown', e=>{ if(e.key==='ArrowDown'){ e.preventDefault(); paletteSel=Math.min(paletteSel+1,paletteItems.length-1); renderPalette(); } else if(e.key==='ArrowUp'){ e.preventDefault(); paletteSel=Math.max(paletteSel-1,0); renderPalette(); } else if(e.key==='Enter'){ e.preventDefault(); palettePick(paletteSel); } else if(e.key==='Escape'){ closePalette(); } });
    document.addEventListener('keydown', e=>{ if((e.metaKey||e.ctrlKey)&&e.key==='k'){ e.preventDefault(); openPalette(); } else if(e.key==='Escape'){ closePalette(); closeCite(); } });

    // keyboard nav: g then letter
    let gPressed=false; document.addEventListener('keydown', e=>{ if(e.target.tagName==='INPUT'||e.target.tagName==='TEXTAREA'||e.target.tagName==='SELECT') return; if(e.key==='g'){ gPressed=true; setTimeout(()=>gPressed=false,800); return; } if(gPressed){ const map={o:'overview',v:'divergence',d:'datasets',s:'signals',c:'cases',h:'health',l:'licences',f:'funding'}; if(map[e.key]){ go(map[e.key]); gPressed=false; } } });


