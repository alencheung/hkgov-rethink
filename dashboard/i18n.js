    // ============ bilingual surface (P-106) ============
    // The API serves zh-HK insight summaries via ?lang=zh-HK (agent/bilingual.rs
    // frame_zh_hk — pure, deterministic). This is the UI half: a 中/EN toggle
    // that (a) re-fetches insights/brief with the lang param and (b) swaps every
    // chrome string via a data-i18n attribute driven by this dictionary.
    //
    // Coverage model: every translatable element carries data-i18n="key".
    // applyI18n() walks [data-i18n] once and sets textContent from the dict —
    // so coverage is exhaustive and auditable (grep data-i18n vs raw English).
    // Evidence values (numbers/dates) are language-neutral and pass through.
    let curLang='en';
    const I18N={
      en:{
        // nav
        nav_overview:'Overview', nav_divergence:'Divergences', nav_datasets:'Datasets', nav_signals:'Signals', nav_cases:'Cases', nav_system:'System', nav_licences:'Licences', nav_funding:'Funding & Credits',
        // overview
        ask_h2:'Ask the agent', ask_ph:'e.g. what is the interbank liquidity doing?', ask_empty:'ask anything about the data — answers carry inline evidence',
        agent_conn:'connecting to the store…', tl_h2:'Cross-source timeline', tl_sub:'— data vs. press releases; gaps glow below',
        silence_h2:'Silence Index', silence_sub:'— opacity, quantified · click to see the exact missing dates',
        // Per-institution source selector.
        silence_source_label:'Institution:', silence_source_aria:'Choose institution',
        silence_src_hkma:'HKMA (Monetary)', silence_src_immigration:'Immigration (Borders)', silence_src_landregistry:'Land Registry (Property)', silence_src_rvd:'R&VD (Rents/Prices)',
        silence_loading:'Loading…', silence_unavail:'Silence Index unavailable', silence_no_signals:'no opacity signals this period',
        silence_label:'Silence Index',
        // timeline empty states
        tl_no_data:'no data warmed yet for this dataset', tl_pick_field:'pick a field', tl_not_enough:'not enough points to plot', tl_chart_unavail:'chart unavailable',
        silence_watch:'Save a watch on this index',
        // PR-001: per-signal push dispatch is intentionally deferred to the
        // P-108 Identity Tier (see crates/agent/src/signal.rs). The watch is
        // still saved, but the button must not promise a push that won't come.
        silence_watch_saved:'saved — push alerts arrive in a later release',
        silence_watch_failed:'could not save (HTTP {code})',
        brief_h2:"Today's brief", insights_h2:'All insights', read_state:'read state syncs to this browser',
        sev_all:'all', sev_critical:'critical', sev_warning:'warning', sev_info:'info',
        no_insights_sev:'no insights at this severity',
        // divergence page
        div_h2:'Proxy Divergence Radar', div_sub:'— two official numbers that should agree, but don\'t',
        div_p:'Overlay two series measuring the same underlying fact. The gap between them is the signal — a disagreement no single-source dashboard can see. Each divergence is a citable insight.',
        div_run:'Detect divergence', div_findings:'Divergence findings', div_coverage:'paired-source coverage: limited — cross-agency pairs fire when data.gov.hk expansion lands',
        div_no_pairs:'These two series only overlap in {n} matching periods — we need at least 4 to judge whether they disagree. Try a different field to line them up.',
        div_methodology_footer:'Under the hood: compared {n} matching periods on <code>{join}</code>. Flags a gap above {thr}%, or a match below 0.6. Mirrors the <code>detect_proxy_divergence</code> detector (min 4 periods).',
        // PR-006b: brief facet chips (were raw English, ignored the 中 toggle).
        facet_severity:'severity', facet_kind:'detector', facet_source:'series', facet_field:'field', facet_direction:'direction', facet_magnitude:'size',
        facet_up:'rising', facet_down:'falling', facet_major:'major (>=30%)', facet_moderate:'moderate (15-30%)', facet_small:'small (<15%)',
        facet_critical:'critical', facet_warning:'warning', facet_info:'info', facet_clear:'clear',
        // datasets page
        ds_h2:'Browse datasets', ds_p:'Every ingested HKGOV source, filterable. Click a tag to search.', ds_search:'search datasets…', ds_all_cat:'all categories', ds_no_match:'no datasets match',
        // signals page
        sig_h2:'Signal subscriptions', sig_p:'Author a detector watch, preview what it would have fired over the last 90 days (deterministic — preview IS what will fire), then save it.',
        sig_q_ph:'e.g. tell me when overnight HIBOR breaks 2.5%', sig_preview:'Preview (last 90 days)', sig_save:'Save signal', sig_preview_box:'Preview results appear here.',
        sig_none:'no signals yet — create one above', sig_dispatch:'dispatch history', sig_loading_disp:'loading dispatch log…', sig_never_fired:'This signal hasn\'t dispatched yet. Preview shows how often it would fire — adjust threshold if it\'s too quiet.',
        sig_needs_feature:'Alert dispatch needs the alerts feature. Your signals are saved and will dispatch once enabled.',
        // PR-Marcus: honest Signals-surface strings (were raw English literals).
        sig_saved_h2:'Saved signals',
        sig_previewing:'previewing (deterministic, last 90 days)…',
        sig_preview_err:'preview error: {detail}',
        sig_fired:'<b>This signal would have fired {n} time{s} in the last {days} days.</b> Detector <code>{det}</code> on <code>{src}</code>.',
        sig_recent:'recent: {list}',
        sig_none_fired:'none — try widening the threshold.',
        sig_could_not_save:'Could not save: {detail}',
        sig_saved_ok:'Saved — push alerts arrive when proactive alerting is enabled.',
        sig_enabled:'enabled', sig_paused:'paused', sig_pause:'pause', sig_enable:'enable', sig_delete:'delete',
        sig_disp_title:'system dispatch log ({n})',
        sig_disp_intro:'Shows recent proactive alerts system-wide (per-signal logs arrive with the identity tier).',
        sig_status_delivered:'delivered', sig_status_bounced:'bounced', sig_status_failed:'failed', sig_status_unknown:'unknown', sig_disp_fallback:'(dispatch)',
        // cases page
        cases_h2:'Drill-in investigations', cases_p:'Saved, resumable, shareable case files. Launch one from any insight\'s Investigate button, or open one below to continue the step-through workspace.',
        cases_none:'no cases yet — open an insight on Overview and click "Investigate"',
        auth_needed_signals:'sign in to view your saved signals',
        auth_needed_cases:'sign in to view your case files',
        // D-018/D-033: in-page magic-link sign-in flow.
        auth_sign_in:'Sign in', auth_signed_in:'Signed in — click to manage', auth_modal_title:'Sign in',
        auth_step1_blurb:'Sign in to save signals, open case files, and watch the Silence Index. A one-time sign-in link will be sent to your email.',
        auth_email:'Email', auth_email_ph:'you@example.com', auth_send_link:'Send sign-in link',
        auth_sending:'sending…', auth_send_failed:'could not send link (HTTP {code})', auth_bad_email:'enter a valid email address',
        auth_link_sent:'link sent — check your email, then paste the token or link below.',
        auth_auto_redeeming:'signing you in…', auth_signed_in_ok:'signed in ✓',
        auth_redeeming:'verifying…', auth_redeem_failed:'sign-in failed — the link may be expired or already used.',
        auth_redeem_failed_code:'sign-in failed (HTTP {code}) — the link may be expired or already used.', auth_no_token:'paste the token or magic-link URL first.',
        auth_manual_blurb:'If your server did not return the link inline (production), paste the token from your email or the full magic-link URL:',
        auth_manual_ph:'token or https://…/auth/redeem?token=…', auth_redeem:'Sign in',
        auth_signed_in_as:'Signed in as', auth_sign_out:'Sign out',
        sig_preview_data_unavailable:'data temporarily unavailable (refresh in progress or cache cold) — retry shortly to see real findings',
        // investigation workspace (inv-* keys; were raw English literals in the innerHTML template).
        inv_seed_insight:'Seed insight:', inv_guided_steps:'guided next steps:',
        inv_chip_related:'related series', inv_chip_parallels:'historical parallels', inv_chip_cross:'cross-source check',
        inv_no_steps:'no steps yet — ask below or click a guided chip', inv_input_ph:'ask a follow-up for this case…',
        // system page
        sys_h2:'System health', sys_p:'Per-source circuit-breaker state + record counts — operational detail. A source showing open or zero records is failing to warm.',
        // licences page
        lic_h2:'Licences, registration & resources', lic_p:'A secondary civic-resource directory. Curated highlights first, then the full directory by issuing department. Every entry links to an official HK source.',
        lic_common:'Most common licences', lic_full:'Full directory by issuing department', lic_search:'filter by department or licence…', lic_no_match:'no department matches your filter',
        // licence card labels
        lic_open_portal:'Open portal', lic_how_apply:'How to apply', lic_licences_label:'Licences:', lic_official:'Official page', lic_choose:'choose a scenario above',
        lic_players:'Related market players', lic_players_disc:'Top named operators — directional, from 2024–25 public sources.',
        lic_note:'Curated directory. For exhaustive lists,', lic_search_blis:'search BLIS',
        // licence wizard
        lic_want_to:'I want to… pick what you want to do.', lic_choose_scenario:'— choose what you want to do —', lic_show_plan:'Show my licence plan',
        // funding & credits page
        fc_h2:'Funding & credits for your business', fc_p:'Government grants, loan guarantees, incubators, and vendor cloud/ad credits — what\'s out there, who it\'s for, and what it\'s worth. Curated as of June 2026; verify current terms on each official page before applying.',
        fc_directory:'Full directory by type', fc_filter_by:'Filter by your business category:', fc_chip_all:'all', fc_clear:'clear',
        fc_want_to:'I am… pick your situation for a tailored support plan.', fc_choose_scenario:'— choose your situation —', fc_show_plan:'Show my support plan', fc_choose:'choose a situation above',
        fc_provider:'Provider', fc_official:'Official page', fc_matches:'<i class="ri-check-line"></i> matches your category', fc_count:'{n} of {t} programmes',
        fc_legend:'Types:', fc_type_grant:'Grant', fc_type_loan:'Loan guarantee', fc_type_incubation:'Incubation', fc_type_investment:'Investment', fc_type_advisory:'Advisory', fc_type_cloud:'Cloud credits', fc_type_ads:'Ad credits',
        fc_cat_all:'all', fc_cat_sme:'Operating SME', fc_cat_tech:'Tech startup', fc_cat_retail:'Retail & F&B', fc_cat_trade:'Trading company', fc_cat_creative:'Creative',
        fc_note:'Curated directory, figures as of June 2026. Programmes change often — always confirm on the official page before applying.', fc_no_match:'no programme matches your filter',
        // form field labels (signals + divergence)
        lbl_source:'source', lbl_dataset:'dataset', lbl_detector:'detector', lbl_field:'field', lbl_threshold:'threshold', lbl_direction:'direction', lbl_cadence:'cadence',
        lbl_above:'above', lbl_below:'below', lbl_describe:'describe it in your words (optional)',
        lbl_primary_source:'primary proxy · source', lbl_primary_dataset:'primary · dataset', lbl_primary_field:'primary · field',
        lbl_companion_source:'companion proxy · source', lbl_companion_dataset:'companion · dataset', lbl_companion_field:'companion · field',
        lbl_join:'join field (how the two series align by period)', lbl_div_threshold:'divergence watch threshold (%)',
        // detector option descriptions
        det_threshold_crossing:'threshold_crossing', det_series_jump:'series_jump (% move)', det_outlier:'outlier (MAD robust z)',
        // datasets table headers
        th_cat:'cat', th_source_dataset:'source / dataset', th_tags:'tags', th_cadence:'cadence', th_records:'records',
        th_all_categories:'all categories',
        // divergence known-pairs
        div_known_pairs:'Known vetted pairs:',
        // timeline legend + gap labels
        tl_data_series:'data series', tl_press_release:'press release', tl_unattributed_move:'unattributed move (the gap)', tl_press_no_data:'press with no data',
        tl_show_press:'press releases', tl_gaps_header:'GAPS — where data and press disagree',
        kind_unattributed:'unattributed move', kind_press_no_data:'press with no data',
        // dataset categories (the 8 Category taxonomy values)
        cat_monetary:'monetary', cat_fiscal:'fiscal', cat_property:'property', cat_trade:'trade', cat_population:'population', cat_livability:'livability', cat_government:'government', cat_other:'other',
        // data source slugs (institution/portal names) — used by the source
        // dropdowns so they show names, not the raw enum tag.
        src_hkma:'HKMA', src_datagovhk:'data.gov.hk', src_press:'HKMA Press', src_landsd:'LandsD / CSDI', src_immigration:'Immigration Department', src_landregistry:'Land Registry', src_rvd:'Rating & Valuation Dept', src_chungsen:'Chung Sen', src_aaproperty:'AA Property', src_hkp:'HK Property (HKP)', src_midland:'Midland',
        // dataset tag chips (cross-cutting concerns on /sources) — used by
        // tagLabel() so chips show a friendly name, not the raw tag slug.
        // Source-slug tags reuse the src_* translations via tagLabel()'s
        // fallback chain, so only the topic-specific tags need explicit entries.
        tag_hibor:'HIBOR', 'tag_interest-rate':'interest rate', tag_property:'property', 'tag_price-index':'price index',
        tag_transactions:'transactions', tag_auction:'auction', tag_foreclosure:'foreclosure', 'tag_bank-owned':'bank-owned',
        'tag_land-registry':'Land Registry', 'tag_transaction-volume':'transaction volume', 'tag_by-class':'by class',
        'tag_rental-index':'rental index', tag_domestic:'domestic', 'tag_passenger-traffic':'passenger traffic',
        'tag_border-crossing':'border crossing', 'tag_control-point':'control point', tag_totals:'totals',
        tag_geospatial:'geospatial', tag_catalog:'catalog', 'tag_economic-indicators':'economic indicators',
        tag_affordability:'affordability', 'tag_mortgage-rate':'mortgage rate', 'tag_recent-sales':'recent sales',
        tag_session:'session', tag_schedule:'schedule', 'tag_中文':'中文', tag_english:'English',
        // cadence values
        cad_daily:'daily', cad_weekly:'weekly', cad_monthly:'monthly', cad_quarterly:'quarterly', cad_annual:'annual', cad_yoy:'year-over-year', cad_unknown:'unknown',
        // record field keys (HKMA/data.gov.hk) — used by the timeline field
        // dropdown so it shows descriptions, not raw snake_case tags. Unknown
        // keys fall through to prettyField()'s humanize path.
        field_hibor_overnight:'HIBOR overnight', field_hibor_1_week:'HIBOR 1-week', field_hibor_1_month:'HIBOR 1-month', field_hibor_3_months:'HIBOR 3-month', field_hibor_6_months:'HIBOR 6-month', field_hibor_12_months:'HIBOR 12-month',
        field_closing_balance:'closing balance', field_opening_balance:'opening balance',
        field_end_of_month:'end of month', field_end_of_date:'end of date', field_end_of_quarter:'end of quarter',
        field_turnover:'turnover', field_total_turnover:'total turnover',
        field_total_reserves:'total reserves', field_reserves:'reserves',
        // keyboard hint
        kbd_hint:'{kbd} command palette · {g} then a tab letter to jump · type {slash} in chat to start an investigation',
        // silence index dynamic strings
        silence_events:'{n} unexplained event{s} this period', silence_methodology:'methodology v{v} · scoped to the selected institution · deterministic · click the score to see the exact dates',
        silence_unavail:'Silence Index unavailable', silence_cannot_reach:'cannot reach /v1/silence-index',
        silence_no_ids:'no per-finding ids (derived signal)', silence_weight:'weight',
        // silence signal kinds (rendered in the breakdown chips)
        sk_press_only_gap:'press only gap', sk_data_only_gap:'data only gap', sk_unattributed_jump:'unattributed jump', sk_missing_data_day:'missing data day',
        // cite modal
        cite_title:'Cite this finding', cite_copy:'Copy', cite_bundle:'JSON bundle + manifest', cite_permalink:'Copy permalink', cite_loading:'loading…',
        // palette
        pal_ph:'Jump to a page, search insights or datasets…', pal_nav:'navigate', pal_open:'open', pal_esc:'esc to close', pal_no_match:'no matches',
        // common buttons / states
        investigate:'Investigate', loading:'loading…', refresh_all:'Refresh all',
        // PR-006: dynamic insight-card action buttons + evidence labels (were
        // hardcoded English, so the 中 toggle left them untranslated).
        cite_btn:'Cite', history_btn:'History', mark_read:'mark read',
        evidence_summary:'evidence ({n}) — side by side', ev_role_previous:'previous', ev_role_current:'current',
        // divergence verdict strings (JS-built, were English-only)
        div_verdict_divergence:'value divergence', div_verdict_decoupling:'decoupling', div_verdict_none:'no divergence',
        div_moved_together:'These two used to move together. Lately they have pulled apart — the match is only {r} across the last {n} matching periods.',
        div_correlation_note:'The {r} is how tightly the two series still move together (1.0 = perfectly in step). A low number means they have stopped tracking each other.',
        div_decoupling_followup:'Often this means one series changed its definition, started lagging the other, or the underlying relationship genuinely shifted.',
        div_proxies_disagree:'Two measures that should agree are {pct}% apart at {date} ({a} vs {b}). That is above the {thr}% watch line.',
        div_proxies_agree:'They agree for now — latest gap is {pct}% (under the {thr}% watch line) and the two are still moving together.',
        // PR-010: first-run orientation (plain language for a cold visitor).
        onboard_msg:'This tool shows where HK government data and press releases disagree. New here? Start with the Silence Index below — it is the one number you can quote.',
        onboard_dismiss:'Dismiss',
        // PR-006 (persona-review): toolbar labels/placeholders/aria — previously
        // raw English that leaked in both languages.
        lbl_api_base:'API base URL', lbl_api_key:'API key',
        ph_api_base:'http://localhost:8080', ph_api_key:'key',
        aria_api_base:'API base URL', aria_api_key:'API key', aria_refresh_all:'Refresh all',
        aria_primary_nav:'Primary', aria_send:'Send', aria_filter_cat:'Filter by category',
        title_agent_mode:'agent mode', title_experimental:'experimental detector', badge_experimental:'experimental',
        title_lang_toggle:'switch language', mode_heuristic:'heuristic',
        brand_title:'HK City Pulse', brand_subtitle:'— what the press room leaves untold',
        // PR-006: agent strip messages (previously hardcoded English).
        agent_failing_open:'scanning, but <b style="color:var(--warn)">{src}</b> circuit is open — serving cached findings',
        agent_watching:'watching HKGOV sources — <b>{n}</b> findings held',
        agent_held_new:'<b>{n}</b> findings held · <span class="newflag" data-action="show-only-new">{delta} new</span> since I last checked',
        agent_held_healthy:'<b>{n}</b> findings held · all sources healthy',
        // PR-006: insight card meta row.
        card_conf:'{n}% conf', card_by:'by {who}',
        // PR-006: unprecedentedness band (previously raw English).
        unprec_label:'unprecedentedness',
        unprec_top_pct:'top {n}%', unprec_extreme:'extreme', unprec_one_in_n:' · 1-in-{n}',
        unprec_in_range:'in normal range', unprec_current_value:'current value: {v}',
        unprec_normal_range:'normal {lo}–{hi}', unprec_min:'min {v}', unprec_max:'max {v}',
        unprec_last_exceeded:'last exceeded {link}',
        // PR-006: comparator (prior parallel) card.
        cmp_badge:'prior parallel', cmp_prior_exceedance:'prior exceedance',
        cmp_beyond_edge:'When this field last exceeded its normal range, it was <b>{value}</b> on <b>{rid}</b>. That exceedance was {pct}% beyond the band edge.',
        cmp_last_exceeded_plain:'When this field last exceeded its normal range, it was <b>{value}</b> on <b>{rid}</b>.',
        // PR-006: return-to-dashboard banner.
        return_new_since:'<b>{n}</b> new since {when}', return_evolved:' · <b>{n}</b> evolved',
        return_show_new:'show only new', return_dismiss:'dismiss',
        // PR-006: honest degraded/empty states (previously a misleading
        // "scans within ~6h" even when the API was unreachable).
        degraded_upstream:'— upstream degraded: {srcs}.',
        degraded_retry:'to retry.',
        degraded_unreachable:'Cannot reach the data API from this host. If you are viewing a static deploy, an API origin may need to be configured.',
        degraded_scans:'The agent scans periodically — new findings appear within ~6h.',
        degraded_banner:'Upstream degraded: {srcs}.',
        degraded_retry_now:'retry now'
      },
      zh:{
        // nav
        nav_overview:'概覽', nav_divergence:'分歧', nav_datasets:'數據集', nav_signals:'訊號', nav_cases:'個案', nav_system:'系統', nav_licences:'牌照', nav_funding:'資助與優惠',
        // overview
        ask_h2:'向代理提問', ask_ph:'例如：銀行同業流動性如何？', ask_empty:'就數據提出任何問題 — 答案附有內嵌證據',
        agent_conn:'正在連接數據庫…', tl_h2:'跨來源時間軸', tl_sub:'— 數據 vs. 新聞稿；分歧於下方標示',
        silence_h2:'沉默指數', silence_sub:'— 不透明程度，量化呈現 · 按此查看確實缺失的日期',
        // 機構來源選擇器。
        silence_source_label:'機構：', silence_source_aria:'選擇機構',
        silence_src_hkma:'金管局（貨幣）', silence_src_immigration:'入境事務處（口岸）', silence_src_landregistry:'土地註冊處（物業）', silence_src_rvd:'差餉物業估價處（租金/樓價）',
        silence_loading:'載入中…', silence_unavail:'沉默指數無法使用', silence_no_signals:'本期無不透明訊號',
        silence_label:'沉默指數',
        // timeline empty states
        tl_no_data:'此數據集尚未暖機', tl_pick_field:'請選擇欄位', tl_not_enough:'數據點不足以繪圖', tl_chart_unavail:'圖表無法使用',
        silence_watch:'儲存此指數的監察',
        // PR-001：按訊號推送排程刻意延後至 P-108 身份層（見 crates/agent/src/signal.rs）。
        // 監察仍會儲存，但按鈕不得承諾尚未實現的推送。
        silence_watch_saved:'已儲存 — 推送通知將於日後版本推出',
        silence_watch_failed:'無法儲存（HTTP {code}）',
        brief_h2:'今日簡報', insights_h2:'所有洞察', read_state:'已讀狀態同步至此瀏覽器',
        sev_all:'全部', sev_critical:'嚴重', sev_warning:'警告', sev_info:'資訊',
        no_insights_sev:'此嚴重程度無洞察',
        // divergence page
        div_h2:'代理分歧雷達', div_sub:'— 兩個本應一致的官方數字，卻不一致',
        div_p:'將兩個衡量同一基礎事實的數列疊加。兩者之間的差距即為訊號 — 單一來源儀表板無法看見的分歧。每項分歧均為可引用的洞察。',
        div_run:'偵測分歧', div_findings:'分歧發現', div_coverage:'配對來源覆蓋：有限 — 跨機構配對於 data.gov.hk 擴展後啟用',
        div_no_pairs:'這兩組數列只有 {n} 個可比對期數重疊 —— 至少需要 4 個才能判斷兩者是否出現分歧。請嘗試用其他欄位將兩者對齊。',
        div_methodology_footer:'技術說明：以 <code>{join}</code> 比對 {n} 個可比對期數。差距高於 {thr}%，或吻合度低於 0.6 時標示。對應 <code>detect_proxy_divergence</code> 偵測器（最少 4 個期數）。',
        // PR-006b：簡報篩選標籤（原先為硬編碼英文，切換 中 時未被翻譯）。
        facet_severity:'嚴重程度', facet_kind:'偵測器', facet_source:'數列', facet_field:'欄位', facet_direction:'方向', facet_magnitude:'幅度',
        facet_up:'上升', facet_down:'下降', facet_major:'重大（>=30%）', facet_moderate:'中等（15-30%）', facet_small:'輕微（<15%）',
        facet_critical:'嚴重', facet_warning:'警告', facet_info:'資訊', facet_clear:'清除',
        // datasets page
        ds_h2:'瀏覽數據集', ds_p:'每個已擷取的港府開放數據來源，可篩選。點擊標籤以搜尋。', ds_search:'搜尋數據集…', ds_all_cat:'所有類別', ds_no_match:'無符合數據集',
        // signals page
        sig_h2:'訊號訂閱', sig_p:'建立偵測器監察，預覽過去 90 天本應觸發的結果（確定性 — 預覽即將觸發），然後儲存。',
        sig_q_ph:'例如：當隔夜拆息突破 2.5% 時通知我', sig_preview:'預覽（過去 90 天）', sig_save:'儲存訊號', sig_preview_box:'預覽結果顯示於此。',
        sig_none:'尚無訊號 — 請在上方建立一個', sig_dispatch:'派發紀錄', sig_loading_disp:'載入派發紀錄…', sig_never_fired:'此訊號尚未派發。預覽顯示觸發頻率 — 如太安靜請調整閾值。',
        sig_needs_feature:'訊號派發需啟用 alerts 功能。您的訊號已儲存，功能啟用後即會派發。',
        // PR-Marcus：訊號介面的誠實字串（原先為硬編碼英文）。
        sig_saved_h2:'已儲存的訊號',
        sig_previewing:'預覽中（確定性，過去 90 天）…',
        sig_preview_err:'預覽錯誤：{detail}',
        sig_fired:'<b>此訊號在過去 {days} 天內本應觸發 {n} 次。</b>偵測器 <code>{det}</code>，對象 <code>{src}</code>。',
        sig_recent:'近期：{list}',
        sig_none_fired:'沒有觸發 — 請嘗試放寬閾值。',
        sig_could_not_save:'無法儲存：{detail}',
        sig_saved_ok:'已儲存 — 主動警示將於警示功能啟用後送達。',
        sig_enabled:'已啟用', sig_paused:'已暫停', sig_pause:'暫停', sig_enable:'啟用', sig_delete:'刪除',
        sig_disp_title:'系統派發紀錄（{n}）',
        sig_disp_intro:'顯示全系統近期的主動警示（逐個訊號的紀錄將隨身份層推出）。',
        sig_status_delivered:'已送達', sig_status_bounced:'退回', sig_status_failed:'失敗', sig_status_unknown:'未知', sig_disp_fallback:'（派發）',
        // cases page
        cases_h2:'深入調查', cases_p:'已儲存、可恢復、可分享的個案檔案。從任何洞察的「調查」按鈕啟動，或於下方開啟以繼續逐步工作區。',
        cases_none:'尚無個案 — 在概覽頁打開洞察並按「調查」',
        auth_needed_signals:'請登入以查看您儲存的訊號',
        auth_needed_cases:'請登入以查看您的個案檔案',
        // D-018/D-033：頁內魔法連結登入流程。
        auth_sign_in:'登入', auth_signed_in:'已登入 — 點擊管理', auth_modal_title:'登入',
        auth_step1_blurb:'登入以儲存訊號、開啟個案檔案，並追蹤沉默指數。一次性登入連結將會發送至您的電郵。',
        auth_email:'電郵', auth_email_ph:'you@example.com', auth_send_link:'傳送登入連結',
        auth_sending:'傳送中…', auth_send_failed:'無法傳送連結（HTTP {code}）', auth_bad_email:'請輸入有效的電郵地址',
        auth_link_sent:'連結已傳送 — 請查看您的電郵，然後在下方貼上權杖或連結。',
        auth_auto_redeeming:'登入中…', auth_signed_in_ok:'已登入 ✓',
        auth_redeeming:'驗證中…', auth_redeem_failed:'登入失敗 — 連結可能已過期或已被使用。',
        auth_redeem_failed_code:'登入失敗（HTTP {code}） — 連結可能已過期或已被使用。', auth_no_token:'請先貼上權杖或魔法連結 URL。',
        auth_manual_blurb:'若您的伺服器未在回應中附上連結（正式環境），請貼上電郵中的權杖或完整的魔法連結 URL：',
        auth_manual_ph:'權杖或 https://…/auth/redeem?token=…', auth_redeem:'登入',
        auth_signed_in_as:'登入身分', auth_sign_out:'登出',
        sig_preview_data_unavailable:'資料暫時無法使用（重新整理進行中或快取為空） — 請稍後重試以查看真實結果',
        // 調查工作區（inv-* 索引鍵；原先為 innerHTML 範本中的原始英文字串）。
        inv_seed_insight:'種子洞察：', inv_guided_steps:'引導式下一步：',
        inv_chip_related:'相關數列', inv_chip_parallels:'歷史對照', inv_chip_cross:'跨源核對',
        inv_no_steps:'尚未有步驟——在下方提問或點選引導選項', inv_input_ph:'就此個案追問…',
        // system page
        sys_h2:'系統健康', sys_p:'各來源斷路器狀態 + 紀錄數量 — 運作細節。顯示 open 或零紀錄的來源未能暖機。',
        // licences page
        lic_h2:'牌照、登記與資源', lic_p:'次要的公民資源目錄。先顯示精選重點，再按發牌部門顯示完整目錄。每項均連結至官方港府來源。',
        lic_common:'最常用牌照', lic_full:'按發牌部門的完整目錄', lic_search:'按部門或牌照篩選…', lic_no_match:'無符合部門',
        lic_open_portal:'開啟入口', lic_how_apply:'申請方法', lic_licences_label:'牌照：', lic_official:'官方頁面', lic_choose:'請於上方選擇情況',
        lic_players:'相關市場參與者', lic_players_disc:'頂尖具名營辦商 — 方向性，取自 2024–25 年公開來源。',
        lic_note:'精選目錄。如需詳盡清單，', lic_search_blis:'搜尋 BLIS',
        // licence wizard
        lic_want_to:'我想… 選擇您要做的事。', lic_choose_scenario:'— 請選擇您要做的事 —', lic_show_plan:'顯示我的牌照計劃',
        // funding & credits page
        fc_h2:'為您的業務而設的資助與優惠', fc_p:'政府資助、貸款擔保、孵化器，以及供應商雲端／廣告優惠 — 了解市場上有甚麼、適合誰、價值多少。截至 2026 年 6 月精選；申請前請於各官方頁面確認最新條款。',
        fc_directory:'按類型瀏覽完整目錄', fc_filter_by:'按您的業務類別篩選：', fc_chip_all:'全部', fc_clear:'清除',
        fc_want_to:'我是… 選擇您的情況以取得度身訂造的支援方案。', fc_choose_scenario:'— 請選擇您的情況 —', fc_show_plan:'顯示我的支援方案', fc_choose:'請於上方選擇情況',
        fc_provider:'提供機構', fc_official:'官方頁面', fc_matches:'<i class="ri-check-line"></i> 符合您的類別', fc_count:'{t} 個計劃中的 {n} 個',
        fc_legend:'類型：', fc_type_grant:'資助', fc_type_loan:'貸款擔保', fc_type_incubation:'孵化', fc_type_investment:'投資', fc_type_advisory:'顧問', fc_type_cloud:'雲端優惠', fc_type_ads:'廣告優惠',
        fc_cat_all:'全部', fc_cat_sme:'營運中小企', fc_cat_tech:'科技初創', fc_cat_retail:'零售及餐飲', fc_cat_trade:'貿易公司', fc_cat_creative:'創意產業',
        fc_note:'精選目錄，數據截至 2026 年 6 月。計劃經常變更 — 申請前請務必於官方頁面確認。', fc_no_match:'沒有符合您篩選的計劃',
        // form field labels (signals + divergence)
        lbl_source:'來源', lbl_dataset:'數據集', lbl_detector:'偵測器', lbl_field:'欄位', lbl_threshold:'閾值', lbl_direction:'方向', lbl_cadence:'週期',
        lbl_above:'高於', lbl_below:'低於', lbl_describe:'用您的話描述（可選）',
        lbl_primary_source:'主要代理 · 來源', lbl_primary_dataset:'主要 · 數據集', lbl_primary_field:'主要 · 欄位',
        lbl_companion_source:'配對代理 · 來源', lbl_companion_dataset:'配對 · 數據集', lbl_companion_field:'配對 · 欄位',
        lbl_join:'連結欄位（兩數列按週期對齊的方式）', lbl_div_threshold:'分歧監察閾值 (%)',
        // detector option descriptions
        det_threshold_crossing:'threshold_crossing', det_series_jump:'series_jump（% 變動）', det_outlier:'outlier（MAD 穩健 z 值）',
        // datasets table headers
        th_cat:'類別', th_source_dataset:'來源 / 數據集', th_tags:'標籤', th_cadence:'週期', th_records:'紀錄數',
        th_all_categories:'所有類別',
        // divergence known-pairs
        div_known_pairs:'已知驗證配對：',
        // timeline legend + gap labels
        tl_data_series:'數據數列', tl_press_release:'新聞稿', tl_unattributed_move:'未解釋變動（分歧）', tl_press_no_data:'有新聞稿但無數據',
        tl_show_press:'新聞稿', tl_gaps_header:'分歧 — 數據與新聞稿不一致之處',
        kind_unattributed:'未解釋變動', kind_press_no_data:'有新聞稿但無數據',
        // dataset categories (the 8 Category taxonomy values)
        cat_monetary:'貨幣', cat_fiscal:'財政', cat_property:'物業', cat_trade:'貿易', cat_population:'人口', cat_livability:'宜居', cat_government:'政府', cat_other:'其他',
        // data source slugs — 機構/平台名稱 (source dropdowns)
        src_hkma:'金管局', src_datagovhk:'資料一線通', src_press:'金管局新聞公報', src_landsd:'地政總署／空間數據', src_immigration:'入境事務處', src_landregistry:'土地註冊處', src_rvd:'差餉物業估價處', src_chungsen:'中誠地產', src_aaproperty:'環亞物業拍賣', src_hkp:'香港置業', src_midland:'美聯物業',
        // 標籤 (tag chips)
        tag_hibor:'港元拆息', 'tag_interest-rate':'利率', tag_property:'物業', 'tag_price-index':'樓價指數',
        tag_transactions:'成交', tag_auction:'拍賣', tag_foreclosure:'銀主盤', 'tag_bank-owned':'銀主盤',
        'tag_land-registry':'土地註冊處', 'tag_transaction-volume':'成交量', 'tag_by-class':'按類別',
        'tag_rental-index':'租金指數', tag_domestic:'私人住宅', 'tag_passenger-traffic':'客流',
        'tag_border-crossing':'口岸', 'tag_control-point':'管制站', tag_totals:'合計',
        tag_geospatial:'地理空間', tag_catalog:'目錄', 'tag_economic-indicators':'經濟指標',
        tag_affordability:'負擔能力', 'tag_mortgage-rate':'按揭利率', 'tag_recent-sales':'近期成交',
        tag_session:'場次', tag_schedule:'時間表', 'tag_中文':'中文', tag_english:'English',
        // cadence values
        cad_daily:'每日', cad_weekly:'每週', cad_monthly:'每月', cad_quarterly:'每季', cad_annual:'每年', cad_yoy:'按年比較', cad_unknown:'未知',
        // record field keys — 中文描述 (timeline field dropdown)
        field_hibor_overnight:'隔夜 HIBOR', field_hibor_1_week:'1 週 HIBOR', field_hibor_1_month:'1 個月 HIBOR', field_hibor_3_months:'3 個月 HIBOR', field_hibor_6_months:'6 個月 HIBOR', field_hibor_12_months:'12 個月 HIBOR',
        field_closing_balance:'期末結餘', field_opening_balance:'期初結餘',
        field_end_of_month:'月末', field_end_of_date:'期末日', field_end_of_quarter:'季末',
        field_turnover:'成交額', field_total_turnover:'總成交額',
        field_total_reserves:'總儲備', field_reserves:'儲備',
        // keyboard hint
        kbd_hint:'{kbd} 指令面板 · 按 {g} 後再按分頁字母以跳轉 · 於對話框輸入 {slash} 開始調查',
        // silence index dynamic strings
        silence_events:'本期 {n} 個未解釋事件', silence_methodology:'方法論 v{v} · 限於所選機構 · 確定性 · 按分數查看確實缺失的日期',
        silence_unavail:'沉默指數無法使用', silence_cannot_reach:'無法連接 /v1/silence-index',
        silence_no_ids:'無逐項發現編號（衍生訊號）', silence_weight:'權重',
        // silence signal kinds (rendered in the breakdown chips)
        sk_press_only_gap:'僅新聞稿分歧', sk_data_only_gap:'僅數據分歧', sk_unattributed_jump:'未解釋跳動', sk_missing_data_day:'缺失數據日',
        // cite modal
        cite_title:'引用此發現', cite_copy:'複製', cite_bundle:'JSON 套件 + 清單', cite_permalink:'複製永久連結', cite_loading:'載入中…',
        // palette
        pal_ph:'跳至頁面、搜尋洞察或數據集…', pal_nav:'導覽', pal_open:'開啟', pal_esc:'esc 關閉', pal_no_match:'無符合項目',
        // common buttons / states
        investigate:'調查', loading:'載入中…', refresh_all:'全部重新整理',
        // PR-006：動態洞察卡操作按鈕 + 證據標籤（原先為硬編碼英文，切換 中 時未被翻譯）。
        cite_btn:'引用', history_btn:'歷史', mark_read:'標記為已讀',
        evidence_summary:'證據（{n}）— 並列對比', ev_role_previous:'上一期', ev_role_current:'本期',
        // 分歧判定字串（由 JS 生成，原先僅英文）
        div_verdict_divergence:'數值分歧', div_verdict_decoupling:'脫鉤', div_verdict_none:'無分歧',
        div_moved_together:'這兩組數據過往同步變動，但近期已經拉開距離 —— 在最近 {n} 個可比對期數中，吻合度僅 {r}。',
        div_correlation_note:'{r} 代表兩組數據仍然多緊密地同步變動（1.0 = 完全同步）。數值偏低，代表兩者已不再如以往般一致。',
        div_decoupling_followup:'這通常意味其中一組數據改變了定義、開始滯後，或兩者之間的關係確實出現了轉變。',
        div_proxies_disagree:'兩個理應一致的指標在 {date} 出現 {pct}% 的差距（{a} 對 {b}），已超出 {thr}% 的監察界線。',
        div_proxies_agree:'目前兩者一致 —— 最新差距為 {pct}%（低於 {thr}% 的監察界線），兩組數據仍然同步。',
        // PR-010：首次到訪導引（為初次使用者以淺白語言撰寫）。
        onboard_msg:'此工具呈現香港政府數據與新聞稿之間的不一致。初次使用？請先看下方的「沉默指數」——那是您唯一可以直接引用的數字。',
        onboard_dismiss:'關閉',
        // PR-006（角色評核）：工具列標籤／佔位字／無障礙標籤——原先為硬編碼英文。
        lbl_api_base:'API 基礎網址', lbl_api_key:'API 金鑰',
        ph_api_base:'http://localhost:8080', ph_api_key:'金鑰',
        aria_api_base:'API 基礎網址', aria_api_key:'API 金鑰', aria_refresh_all:'全部重新整理',
        aria_primary_nav:'主導覽', aria_send:'送出', aria_filter_cat:'按類別篩選',
        title_agent_mode:'代理模式', title_experimental:'實驗性偵測器', badge_experimental:'實驗性',
        title_lang_toggle:'切換語言', mode_heuristic:'啟發式',
        brand_title:'HK City Pulse', brand_subtitle:'—— 新聞發布室未言明之處',
        // PR-006：代理狀態列訊息（原先為硬編碼英文）。
        agent_failing_open:'掃描中，但 <b style="color:var(--warn)">{src}</b> 斷路器為開啟狀態 —— 正提供快取發現',
        agent_watching:'監察港府來源中 —— 持有 <b>{n}</b> 項發現',
        agent_held_new:'持有 <b>{n}</b> 項發現 · 自上次查看起新增 <span class="newflag" data-action="show-only-new">{delta} 項</span>',
        agent_held_healthy:'持有 <b>{n}</b> 項發現 · 所有來源運作正常',
        // PR-006：洞察卡片資訊列。
        card_conf:'信心 {n}%', card_by:'由 {who}',
        // PR-006：前所未見程度帶狀圖（原先為硬編碼英文）。
        unprec_label:'前所未見程度',
        unprec_top_pct:'最高 {n}%', unprec_extreme:'極端', unprec_one_in_n:' · {n} 次一遇',
        unprec_in_range:'處於正常範圍', unprec_current_value:'當前數值：{v}',
        unprec_normal_range:'正常 {lo}–{hi}', unprec_min:'最低 {v}', unprec_max:'最高 {v}',
        unprec_last_exceeded:'上次超出於 {link}',
        // PR-006：比較器（前期平行）卡片。
        cmp_badge:'前期平行', cmp_prior_exceedance:'前期超出',
        cmp_beyond_edge:'此欄位上次超出正常範圍時，數值為 <b>{value}</b>，出現於 <b>{rid}</b>。該次超出幅度為波段邊界的 {pct}%。',
        cmp_last_exceeded_plain:'此欄位上次超出正常範圍時，數值為 <b>{value}</b>，出現於 <b>{rid}</b>。',
        // PR-006：返回儀表板橫幅。
        return_new_since:'自 {when} 起新增 <b>{n}</b> 項', return_evolved:' · <b>{n}</b> 項已演變',
        return_show_new:'僅顯示新增', return_dismiss:'關閉',
        // PR-006：誠實的降級／空狀態（原先即使 API 無法連接，仍誤稱「約 6 小時內掃描」）。
        degraded_upstream:'—— 上游降級：{srcs}。',
        degraded_retry:'以重試。',
        degraded_unreachable:'無法從此主機連接數據 API。若您瀏覽的是靜態部署，可能需要設定 API 來源網址。',
        degraded_scans:'代理會定期掃描 —— 新發現約於 6 小時內出現。',
        degraded_banner:'上游降級：{srcs}。',
        degraded_retry_now:'立即重試'
      }
    };
    function t(k, vars){ let s=(I18N[curLang]&&I18N[curLang][k])||I18N.en[k]||k; if(vars){ for(const kk in vars) s=s.replace('{'+kk+'}', vars[kk]); } return s; }
    function langParam(){ return curLang==='zh'?'&lang=zh-HK':''; }
    function toggleLang(){ curLang = curLang==='zh'?'en':'zh'; document.getElementById('langLabel').textContent = curLang==='zh'?'EN':'中'; // PR-011: keep <html lang> in sync so assistive tech announces the right language.
      document.documentElement.lang = curLang==='zh'?'zh-HK':'en'; // re-translate the icon-only close button's aria-label when language flips.
      const ob=document.querySelector('#onboardBanner .onboard-close'); if(ob) ob.setAttribute('aria-label', t('onboard_dismiss')); try{localStorage.setItem(LS_LANG,curLang);}catch(e){} applyI18n(); // re-render dynamic content so structured data follows the toggle
      const wizSel=document.getElementById('wizSelect'); if(wizSel){ wizSel.options.length=1; } renderLicences();
      const fundSel=document.getElementById('fundWizSelect'); if(fundSel){ fundSel.options.length=1; } renderFund();
      if(lastTab==='datasets'){ loadCategories(); loadSources(); } // re-render table + category dropdown with translated labels
      if(lastTab==='overview'){ renderTimeline(); } // re-render timeline legend gaps
      if(lastTab==='divergence' && dvInited){ runDivergence(); } // PR-006: re-render divergence findings so verdict strings follow the toggle
      loadAll(); }
    // Single-pass i18n: walk every [data-i18n] element and set its text from the
    // dictionary. Placeholders use [data-i18n-ph]. This is exhaustive — adding a
    // string means adding data-i18n to the element + a dict entry, nothing else.
    function applyI18n(){
      const lang=I18N[curLang]||I18N.en;
      document.querySelectorAll('[data-i18n]').forEach(el=>{ const k=el.getAttribute('data-i18n'); const v=lang[k]||I18N.en[k]; if(v!=null){ // preserve any trailing count span on nav buttons
        const countSpan=el.querySelector('.count, .new-pip'); el.textContent=v; if(countSpan) el.appendChild(countSpan); } });
      document.querySelectorAll('[data-i18n-ph]').forEach(el=>{ const k=el.getAttribute('data-i18n-ph'); const v=lang[k]||I18N.en[k]; if(v!=null) el.placeholder=v; });
      // aria-label / title translation (AGENT.md §i18n: aria-labels are
      // user-visible and must resolve through the dictionary, just like text).
      // `data-i18n-aria="key"` sets the element's aria-label from the dict;
      // `data-i18n-title="key"` sets its title attribute.
      document.querySelectorAll('[data-i18n-aria]').forEach(el=>{ const k=el.getAttribute('data-i18n-aria'); const v=lang[k]||I18N.en[k]; if(v!=null) el.setAttribute('aria-label',v); });
      document.querySelectorAll('[data-i18n-title]').forEach(el=>{ const k=el.getAttribute('data-i18n-title'); const v=lang[k]||I18N.en[k]; if(v!=null) el.setAttribute('title',v); });
      // Keyboard hint carries inline <b>/<code> tags, so render it as HTML with
      // the kbd/g/slash placeholders preserved (a plain textContent swap would
      // strip the formatting).
      const kh=document.getElementById('kbdHint'); if(kh){ kh.innerHTML='<i class="ri-keyboard-line"></i> '+t('kbd_hint',{kbd:'<b>Cmd/Ctrl+K</b>',g:'<b>g</b>',slash:'<code>/</code>'}); }
    }
    // restore lang preference (browser-language aware)
    try { const saved=localStorage.getItem(LS_LANG); if(saved==='zh'){ curLang='zh'; document.getElementById('langLabel').textContent='EN'; document.documentElement.lang='zh-HK'; } else if(!saved && navigator.language && navigator.language.toLowerCase().startsWith('zh')){ curLang='zh'; document.getElementById('langLabel').textContent='EN'; document.documentElement.lang='zh-HK'; } } catch(e){}


