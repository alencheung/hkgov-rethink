    // ============ boot ============
    async function loadAll(){ persistConfig(); await loadHealthQuiet(); await Promise.all([loadSilence(), loadTimeline(), loadBrief(), loadInsights()]); renderDegradedBanner(); renderReturnBanner(); loadSignals(); loadCases(); updateAgentStrip(allInsights.length, cachedHealth); }
    // D-027 fix: default the base URL to the SAME ORIGIN that served this page
    // whenever it was served over http(s). The prior condition required a
    // truthy `location.port`, which is empty on standard ports (80/443) — so a
    // production API-served `/dashboard` deploy silently fell back to the
    // hardcoded `DEFAULT_API_BASE` (a specific Railway URL) and pointed the
    // browser at a *different* host. Same-origin (empty base) is correct for
    // any deploy where hkgov-api serves the dashboard itself (local dev, the
    // Docker image, or a Railway/cloud deploy visited directly). A separately-
    // hosted dashboard still overrides via the header input or localStorage.
    if(location.protocol.startsWith('http')){ document.getElementById('baseUrl').value=`${location.protocol}//${location.host}`; }
    restoreConfig();
    applyI18n(); // translate chrome to the restored/browser language on first paint
    if(typeof refreshAuthModal==='function') refreshAuthModal(); // D-033: reflect saved session in header
    const initTab=(location.hash||'').replace('#','').split('/')[0];
    if(['overview','datasets','divergence','signals','cases','health','licences','funding'].includes(initTab)) go(initTab);
    loadAll();
    applyI18n(); // re-apply after dynamic content (empty states) may have rendered
    showOnboardIfFirstRun(); // PR-010: orient a first-run visitor (overview)
    checkShareLanding();
    // Pulse: poll + announce new findings (replaces the silent interval)
    setInterval(()=>{ if(lastTab==='overview'){ loadSilence(); loadInsights(); loadHealthQuiet().then(()=>{ renderDegradedBanner(); updateAgentStrip(allInsights.length, cachedHealth); }); } }, 30000);
    // stamp visit on unload so the next session shows the right delta
    window.addEventListener('beforeunload', stampVisit);
    // ============ event delegation (replaces inline on* handlers for CSP) ============
    // All former onclick/onchange/oninput/onkeydown handlers now use data-action.
    // One listener per event type dispatches to the action by attribute name.
    // `el` is the element carrying data-action (== e.currentTarget via closest()).
    function bindDelegation(){
      const A = {
        'go':                     el => go(el.dataset.tab),
        'load-all':               ()  => loadAll(),
        'open-palette':           ()  => openPalette(),
        'close-palette':          ()  => closePalette(),
        'close-palette-on-backdrop': (el, ev) => { if(ev.target === el) closePalette(); },
        'toggle-lang':            ()  => toggleLang(),
        'dismiss-onboard':        ()  => dismissOnboard(),
        'toggle-silence-breakdown': () => toggleSilenceBreakdown(),
        'watch-silence-index':    ()  => watchSilenceIndex(),
        'run-divergence':         ()  => runDivergence(),
        'dv-preset':              el => dvPreset(el.dataset.which),
        'dv-fill-fields':         el => dvFillFields(el.dataset.which),
        'preview-signal':         ()  => previewSignal(),
        'save-signal':            ()  => saveSignal(),
        'sig-dataset-fill':       ()  => sigDatasetFill(),
        'run-wizard':             ()  => runWizard(),
        'run-fund-wizard':        ()  => runFundWizard(),
        'load-timeline':          ()  => loadTimeline(),
        'render-timeline':        ()  => renderTimeline(),
        'load-silence':           ()  => loadSilence(),
        'load-sources':           ()  => loadSources(),
        'render-licences':        ()  => renderLicences(),
        'render-palette':         ()  => renderPalette(),
        'set-sev-filter':         el => setSevFilter(el.dataset.sev),
        'set-fmt':                el => setFmt(el.dataset.fmt),
        'ask-agent':              ()  => askAgent(),
        'explain-and-ask':        el => { document.getElementById('askInput').value='explain the '+el.dataset.kind+' on '+el.dataset.date; askAgent(); go('overview'); },
        'close-cite':             ()  => closeCite(),
        'close-cite-on-backdrop': (el, ev) => { if(ev.target === el) closeCite(); },
        'copy-cite':              ()  => copyCite(),
        'cite-bundle':            ()  => citeBundle(),
        'copy-permalink':         ()  => copyPermalink(),
        'clear-brief-filters':    ()  => clearBriefFilters(),
        'toggle-brief-filter':    el => toggleBriefFilter(el.dataset.filter, el.dataset.value),
        'show-only-new':          ()  => showOnlyNew(),
        'dismiss-parent':         el => { if(el.parentElement) el.parentElement.remove(); },
        'noop':                   ()  => {},
        'vote':                   el => vote(el.dataset.id, el.dataset.useful === 'true', el),
        'open-cite':              el => openCite(el.dataset.id),
        'open-auth':              ()  => openAuth(),
        'close-auth':             ()  => closeAuth(),
        'close-auth-on-backdrop': (el, ev) => { if(ev.target === el) closeAuth(); },
        'send-auth-link':         ()  => sendAuthLink(),
        'redeem-auth-token':      ()  => redeemAuthToken(),
        'sign-out':               ()  => signOut(),
        'mark-read':              el => { markRead(el.dataset.id); const c=el.closest('.card'); if(c) c.classList.remove('unread'); },
        'load-history':           el => loadHistory(el.dataset.id, el),
        'investigate':            el => investigate(el.dataset.id, el.dataset.source, el.dataset.dataset, el.dataset.title),
        'open-inv-workspace':     el => openInvWorkspace(el.dataset.id),
        'close-investigation':    ()  => { activeInvId=null; document.getElementById('invWorkspaceMount').innerHTML=''; },
        'inv-ask':                ()  => invAsk(),
        'inv-ask-on-enter':       (el, ev) => { if(ev.key === 'Enter') invAsk(); },
        'inv-chip':               el => invChip(el.dataset.prompt),
        'del-signal':             el => delSignal(el.dataset.id),
        'toggle-signal':          el => toggleSignal(el.dataset.id, el.dataset.enable === 'true'),
        'load-dispatch-log':      el => loadDispatchLog(el.dataset.id, el),
        'tag-search':             el => tagSearch(el.dataset.tag),
        'palette-pick':           el => palettePick(Number(el.dataset.idx)),
        'set-fund-filter':        el => setFundFilter(el.dataset.cat),
        'open-comparator':        el => openComparator(el.dataset.src, el.dataset.ds, el.dataset.fld, el.dataset.record, Number(el.dataset.value)),
        'open-ext':               el => openExt(el.dataset.url),
      };

      function dispatch(ev){
        const el = ev.target.closest && ev.target.closest('[data-action]');
        if(!el) return;
        const fn = A[el.dataset.action];
        if(fn){ fn(el, ev); }
      }
      // click: open-ext anchors must also cancel the href navigation (was `return false`)
      document.addEventListener('click', ev => {
        const el = ev.target.closest && ev.target.closest('[data-action]');
        if(!el) return;
        const fn = A[el.dataset.action];
        if(!fn) return;
        fn(el, ev);
        if(el.dataset.action === 'open-ext') ev.preventDefault();
      });
      document.addEventListener('change', dispatch);
      document.addEventListener('input', dispatch);
      document.addEventListener('keydown', dispatch);
    }
    bindDelegation();
