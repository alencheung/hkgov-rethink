    // ============ licences (kept, demoted) ============
    function openExt(url){ const w=window.open(url,'_blank','noopener,noreferrer'); if(!w) window.location.href=url; return false; }
    function extBtn(url,label){ return `<a class="ext-link" href="${escapeHtml(url)}" target="_blank" rel="noopener noreferrer" data-action="open-ext" data-url="${escapeHtml(url)}"><i class="ri-arrow-right-up-line"></i> ${escapeHtml(label)}</a>`; }
    // Bilingual licence directory. Each text field carries {en, zh}; L() picks the
    // active language. Licence/permit proper names keep their English form in both
    // languages (they are official terms), but descriptions, roles, and labels
    // translate fully.
    function L(f){ if(f==null) return ''; return (typeof f==='object')?(f[curLang]||f.en):String(f); }
    // Department short-code -> full bilingual name. The card keeps the code as a
    // stable search key but renders the full translated name (HKMA -> 香港金融管理局).
    function deptName(code){ const m=DEPT_NAMES[code]; return m?L(m):code; }
    const DEPT_NAMES={
      'HKMA':{en:'HKMA (Hong Kong Monetary Authority)',zh:'金管局（香港金融管理局）'},
      'SFC':{en:'SFC (Securities & Futures Commission)',zh:'證監會（證券及期貨事務監察委員會）'},
      'IA':{en:'IA (Insurance Authority)',zh:'保監局（保險業監管局）'},
      'FEHD':{en:'FEHD (Food & Environmental Hygiene Dept)',zh:'食環署（食物環境衞生署）'},
      'CR':{en:'CR (Companies Registry)',zh:'公司註冊處'},
      'IRD':{en:'IRD (Inland Revenue Department)',zh:'稅務局'},
      'OFCA':{en:'OFCA (Office of the Communications Authority)',zh:'通訊辦（通訊事務管理局辦公室）'},
      'C&ED':{en:'C&ED (Customs & Excise Department)',zh:'海關（香港海關）'},
      'EMSD':{en:'EMSD (Electrical & Mechanical Services Dept)',zh:'機電署（機電工程署）'},
      'LD':{en:'LD (Labour Department)',zh:'勞工處'},
      'TD':{en:'TD (Transport Department)',zh:'運輸署'},
      'TIA':{en:'TIA (Travel Industry Authority)',zh:'旅監局（旅行業監管局）'}
    };
    const LIC_SCENARIOS=[
      {id:'restaurant',label:{en:'Open a restaurant / food business',zh:'開設餐廳 / 食物業'},summary:{en:'Food Business Licence is the core. Company + premises come first; the licence is issued after fit-out.',zh:'食物業牌照為核心。先辦公司 + 確認處所；裝修後發牌。'},steps:[
        {lic:{en:'Certificate of Incorporation',zh:'公司註冊證書'},who:{en:'Companies Registry',zh:'公司註冊處'},when:{en:'first — register your company',zh:'首先 — 註冊公司'},url:'https://www.cr.gov.hk/'},
        {lic:{en:'Business Registration Certificate',zh:'商業登記證'},who:{en:'Inland Revenue Dept',zh:'稅務局'},when:{en:'with incorporation (issued jointly)',zh:'與註冊同步（同步發出）'},url:'https://www.gov.hk/en/business/registration/license/index.htm'},
        {lic:{en:'Food Business Licence',zh:'食物業牌照'},who:{en:'FEHD',zh:'食環署'},when:{en:'core licence — apply with layout plans; FEHD inspects after fit-out',zh:'核心牌照 — 提交圖則申請；裝修後巡查'},url:'https://www.fehd.gov.hk/english/licensing/guide.html'},
        {lic:{en:'Liquor Licence (if selling alcohol)',zh:'酒牌（如售賣酒精）'},who:{en:'Liquor Licensing Board',zh:'酒牌局'},when:{en:'only if selling alcohol (>1.2% ABV)',zh:'僅當售賣酒精（>1.2% 酒精）'},url:'https://www.fehd.gov.hk/english/licensing/index.html'}
      ]},
      {id:'ecommerce',label:{en:'Run an online store / e-commerce',zh:'經營網店 / 電子商務'},summary:{en:'No single "e-commerce licence" in HK. You need a Business Registration, then sector licences only for what you sell (food, medicine, alcohol). Two regimes still bite: the Trade Descriptions Ordinance and the Personal Data Privacy Ordinance.',zh:'香港沒有單一「電商牌照」。需商業登記，再按所售貨物（食物、藥物、酒精）申請特定牌照。兩項法規仍然適用：《商品說明條例》及《個人資料（私隱）條例》。'},steps:[
        {lic:{en:'Business Registration Certificate',zh:'商業登記證'},who:{en:'Inland Revenue Dept',zh:'稅務局'},when:{en:'first — even online-only businesses need a BR',zh:'首先 — 即使純網上經營亦需商業登記'},url:'https://www.gov.hk/en/business/registration/license/index.htm'},
        {lic:{en:'Sector licence only if goods are regulated',zh:'僅當貨物受管制時需行業牌照'},who:{en:'varies (FEHD food, DH pharmacy, Customs dutiable)',zh:'視情況（食環署食物、衛生署藥物、海關應課稅品）'},when:{en:'e.g. food, restricted medicines, alcohol, tobacco',zh:'例如食物、受管制藥物、酒精、煙草'},url:'https://www.success.tid.gov.hk/tid/eng/blics/index.jsp'},
        {lic:{en:'Trade Descriptions Ordinance compliance',zh:'《商品說明條例》合規'},who:{en:'Customs & Excise (enforces)',zh:'海關（執法）'},when:{en:'applies fully to online retail — no false/misleading ads, no bait pricing',zh:'完全適用於網上零售 — 不得虛假/誤導廣告、不得誘餌定價'},url:'https://www.customs.gov.hk/'},
        {lic:{en:'Personal Data (Privacy) Ordinance compliance',zh:'《個人資料（私隱）條例》合規'},who:{en:'Privacy Commissioner (PCPD)',zh:'私隱專員（公署）'},when:{en:'mandatory for any customer data — consent + opt-out for direct marketing',zh:'處理客戶資料必須遵守 — 須同意 + 直銷可退出'},url:'https://www.pcpd.org.hk/english/data_privacy_law/ordinance_at_a_Glance/ordinance.html'}
      ]},
      {id:'soloentrepreneur',label:{en:'Start as a solo entrepreneur / freelancer',zh:'以個人創業者 / 自由工作者身份創業'},summary:{en:'A freelancer needs a Business Registration and must file profits tax. If you hire anyone (even part-time), MPF + Employees Compensation Insurance become mandatory. No sector licence unless your skill is regulated (e.g. electrician, insurance agent).',zh:'自由工作者需商業登記並申報利得稅。如僱用任何人（即使是兼職），強積金 + 僱員補償保險即為強制。除非技能受規管（如電工、保險代理），否則無需行業牌照。'},steps:[
        {lic:{en:'Business Registration Certificate',zh:'商業登記證'},who:{en:'Inland Revenue Dept',zh:'稅務局'},when:{en:'first — register as a sole proprietorship (simplest form)',zh:'首先 — 以獨資形式登記（最簡單）'},url:'https://www.gov.hk/en/business/registration/license/index.htm'},
        {lic:{en:'Profits Tax filing',zh:'利得稅申報'},who:{en:'Inland Revenue Dept',zh:'稅務局'},when:{en:'annual — file your business profits with IRD',zh:'每年 — 向稅務局申報業務利潤'},url:'https://www.gov.hk/en/residents/taxes/taxfiling/individual/'},
        {lic:{en:'MPF registration (once you hire staff)',zh:'強積金登記（一旦僱用員工）'},who:{en:'MPFA',zh:'積金局'},when:{en:'mandatory within 60 days of hiring anyone — even part-time',zh:'僱用任何人後 60 天內強制辦理 — 即使是兼職'},url:'https://www.mpfa.org.hk/'},
        {lic:{en:'Employees Compensation Insurance (once you hire)',zh:'僱員補償保險（一旦僱用）'},who:{en:'insurer',zh:'保險公司'},when:{en:'mandatory the moment you have an employee',zh:'有員工即須強制投保'},url:'https://www.labour.gov.hk/eng/news/content/20190520.htm'}
      ]},
      {id:'saas',label:{en:'Run a SaaS / software company',zh:'經營 SaaS / 軟件公司'},summary:{en:'Pure software, SaaS, and cloud services do NOT need a telecoms licence. You only cross into OFCA licensing if you provide carriage (network/internet access). Watch the PDPO for user data.',zh:'純軟件、SaaS 及雲端服務無需電訊牌照。僅當提供傳輸（網絡/互聯網接入）時才需 OFCA 牌照。注意用戶資料的《私隱條例》。'},steps:[
        {lic:{en:'Certificate of Incorporation + Business Registration',zh:'公司註冊證書 + 商業登記證'},who:{en:'Companies Registry / IRD',zh:'公司註冊處 / 稅務局'},when:{en:'first — incorporate a limited company',zh:'首先 — 註冊有限公司'},url:'https://www.cr.gov.hk/'},
        {lic:{en:'OFCA telecoms licence — usually NOT required',zh:'OFCA 電訊牌照 — 通常無需'},who:{en:'Office of the Communications Authority',zh:'通訊辦'},when:{en:'only if you provide carriage (network/internet access). Pure SaaS/cloud are exempt',zh:'僅當提供傳輸（網絡/互聯網接入）。純 SaaS/雲端獲豁免'},url:'https://www.ofca.gov.hk/en/industry_focus/regulations_licensing/index.html'},
        {lic:{en:'Personal Data (Privacy) Ordinance compliance',zh:'《個人資料（私隱）條例》合規'},who:{en:'Privacy Commissioner (PCPD)',zh:'私隱專員（公署）'},when:{en:'mandatory — SaaS handles user data by definition',zh:'強制 — SaaS 本質上處理用戶資料'},url:'https://www.pcpd.org.hk/english/data_privacy_law/ordinance_at_a_Glance/ordinance.html'},
        {lic:{en:'MPF + Employees Compensation Insurance',zh:'強積金 + 僱員補償保險'},who:{en:'MPFA / insurer',zh:'積金局 / 保險公司'},when:{en:'once you hire staff',zh:'一旦僱用員工'},url:'https://www.mpfa.org.hk/'}
      ]},
      {id:'consulting',label:{en:'Open a consulting / professional services firm',zh:'開設顧問 / 專業服務公司'},summary:{en:'General business consulting needs only a BR + incorporation. But if you advise in a regulated field — securities, insurance, accounting — you need that profession\'s licence. A company also owes MPF + EC insurance once it hires.',zh:'一般商業顧問僅需商業登記 + 公司註冊。但如在受規管領域（證券、保險、會計）提供意見，則需該專業牌照。公司一旦僱用即須辦強積金 + 僱員補償保險。'},steps:[
        {lic:{en:'Certificate of Incorporation + Business Registration',zh:'公司註冊證書 + 商業登記證'},who:{en:'Companies Registry / IRD',zh:'公司註冊處 / 稅務局'},when:{en:'first',zh:'首先'},url:'https://www.cr.gov.hk/'},
        {lic:{en:'Sector licence only if advising in a regulated field',zh:'僅當在受規管領域提供意見時需行業牌照'},who:{en:'SFC / IA / HKICPA (as applicable)',zh:'證監會 / 保監局 / 會計師公會（視情況）'},when:{en:'securities advice: SFC; insurance advice: IA; audit: HKICPA',zh:'證券意見：證監會；保險意見：保監局；核數：會計師公會'},url:'https://www.success.tid.gov.hk/tid/eng/blics/index.jsp'},
        {lic:{en:'Personal Data (Privacy) Ordinance compliance',zh:'《個人資料（私隱）條例》合規'},who:{en:'Privacy Commissioner (PCPD)',zh:'私隱專員（公署）'},when:{en:'mandatory — you hold client data',zh:'強制 — 您持有客戶資料'},url:'https://www.pcpd.org.hk/english/data_privacy_law/ordinance_at_a_Glance/ordinance.html'},
        {lic:{en:'MPF + Employees Compensation Insurance',zh:'強積金 + 僱員補償保險'},who:{en:'MPFA / insurer',zh:'積金局 / 保險公司'},when:{en:'once you hire staff',zh:'一旦僱用員工'},url:'https://www.mpfa.org.hk/'}
      ]},
      {id:'ewallet',label:{en:'Launch an e-wallet / payment / stored-value facility',zh:'推出電子錢包 / 支付 / 儲值支付工具'},summary:{en:'Issuing stored value (e-wallets, prepaid cards) needs an HKMA licence under Cap. 584. The bar is high: HK$25M minimum paid-up capital, fit-and-proper controllers. An SVF register is published by the HKMA.',zh:'發行儲值（電子錢包、預付卡）需根據《第584章》取得金管局牌照。門檻高：最低實繳資本 2,500 萬港元，控制人須為適當人選。金管局公布 SVF 登記冊。'},steps:[
        {lic:{en:'Certificate of Incorporation + Business Registration',zh:'公司註冊證書 + 商業登記證'},who:{en:'Companies Registry / IRD',zh:'公司註冊處 / 稅務局'},when:{en:'first',zh:'首先'},url:'https://www.cr.gov.hk/'},
        {lic:{en:'Stored Value Facility (SVF) licence',zh:'儲值支付工具（SVF）牌照'},who:{en:'Hong Kong Monetary Authority',zh:'金管局'},when:{en:'core — Cap. 584; min. HK$25M paid-up capital; controllers must be fit & proper',zh:'核心 — 第584章；最低實繳資本 2,500 萬港元；控制人須為適當人選'},url:'https://www.hkma.gov.hk/eng/regulatory-resources/authorization-licensing-and-approval/'},
        {lic:{en:'Personal Data (Privacy) Ordinance compliance',zh:'《個人資料（私隱）條例》合規'},who:{en:'Privacy Commissioner (PCPD)',zh:'私隱專員（公署）'},when:{en:'mandatory — payment data is sensitive',zh:'強制 — 支付資料屬敏感資料'},url:'https://www.pcpd.org.hk/english/data_privacy_law/ordinance_at_a_Glance/ordinance.html'}
      ]},
      {id:'importexport',label:{en:'Import / export dutiable or controlled goods',zh:'進出口應課稅或受管制貨品'},summary:{en:'Customs & Excise licences for dutiable commodities (liquor, tobacco, hydrocarbon oil, methanol); other controlled goods may need additional permits. Textile exports need trader registration.',zh:'海關就應課稅品（酒類、煙草、碳氫油、甲醇）發牌；其他受管制貨品可能需額外許可證。紡織品出口需貿易商註冊。'},steps:[
        {lic:{en:'Certificate of Incorporation + Business Registration',zh:'公司註冊證書 + 商業登記證'},who:{en:'Companies Registry / IRD',zh:'公司註冊處 / 稅務局'},when:{en:'first',zh:'首先'},url:'https://www.cr.gov.hk/'},
        {lic:{en:'Dutiable Commodities Licence',zh:'應課稅品牌照'},who:{en:'Customs & Excise',zh:'海關'},when:{en:'core for dutiable goods (liquor, tobacco, oil, methanol)',zh:'應課稅品核心牌照（酒類、煙草、油、甲醇）'},url:'https://www.customs.gov.hk/en/service-enforcement-information/trade-facilitation/dutiable-commodities/about-licences/index.html'},
        {lic:{en:'Import & Export Licence / Textile Trader Registration',zh:'進出口牌照 / 紡織品貿易商註冊'},who:{en:'Trade & Industry Dept',zh:'工業貿易署'},when:{en:'for controlled goods or textile exports',zh:'用於受管制貨品或紡織品出口'},url:'https://www.tid.gov.hk/'}
      ]}
    ];
    const LIC_KEY=[{t:'BLIS',u:'https://www.success.tid.gov.hk/tid/eng/blics/index.jsp',d:{en:'Authoritative licence search.',zh:'權威牌照搜尋。'}},{t:{en:'Online Licence Services',zh:'網上牌照服務'},u:'https://www.licensing.gov.hk/',d:{en:'Apply/renew across departments.',zh:'跨部門申請/續期。'}},{t:{en:'GovHK Licensing',zh:'GovHK 牌照'},u:'https://www.gov.hk/en/business/registration/license/index.htm',d:{en:'Gateway topic page.',zh:'入門主題頁。'}}];
    const LIC_HIGHLIGHTS=[{name:{en:'Business Registration',zh:'商業登記'},dept:{en:'Inland Revenue Dept',zh:'稅務局'},url:'https://www.gov.hk/en/business/registration/license/index.htm'},{name:{en:'Company Incorporation',zh:'公司註冊'},dept:{en:'Companies Registry',zh:'公司註冊處'},url:'https://www.cr.gov.hk/'},{name:{en:'Food Business Licence',zh:'食物業牌照'},dept:{en:'FEHD',zh:'食環署'},url:'https://www.fehd.gov.hk/english/licensing/index.html'}];
    const LIC_DEPARTMENTS=[
      {dept:'HKMA',deptUrl:'https://www.hkma.gov.hk/eng/',desc:{en:'The three-tier banking system + stored value facilities.',zh:'三級銀行體系 + 儲值支付工具。'},licences:[{en:'Licensed Bank',zh:'持牌銀行'},{en:'Restricted Licence Bank',zh:'有限制牌照銀行'},{en:'Deposit-taking Company',zh:'接受存款公司'},{en:'SVF issuer',zh:'儲值支付工具發行人'}],portal:{label:{en:'Authorization & licensing',zh:'認可與發牌'},url:'https://www.hkma.gov.hk/eng/regulatory-resources/authorization-licensing-and-approval/'},code:'HKMA'},
      {dept:'SFC',deptUrl:'https://www.sfc.hk/en/',desc:{en:'Securities & futures regulated activities.',zh:'證券及期貨受規管活動。'},licences:[{en:'Licensed Corporation',zh:'持牌法團'},{en:'Licensed Representative',zh:'持牌代表'},{en:'VATP',zh:'虛擬資產交易平台'}],portal:{label:{en:'Licensing',zh:'發牌'},url:'https://www.sfc.hk/en/Regulatory-functions/Intermediaries/Licensing'},code:'SFC'},
      {dept:'IA',deptUrl:'https://www.ia.org.hk/',desc:{en:'Insurance intermediaries & authorized insurers.',zh:'保險中介人及獲授權保險人。'},licences:[{en:'Licensed Insurance Agent',zh:'持牌保險代理人'},{en:'Licensed Insurance Broker Company',zh:'持牌保險經紀公司'},{en:'Authorized Insurer',zh:'獲授權保險人'}],portal:{label:{en:'Licence application',zh:'牌照申請'},url:'https://www.ia.org.hk/en/supervision/reg_ins_intermediaries/licence_application.html'},code:'IA'},
      {dept:'FEHD',deptUrl:'https://www.fehd.gov.hk/english/licensing/index.html',desc:{en:'Largest trade-licence issuer — food, markets, pools.',zh:'最大貿易牌照簽發機構 — 食物、街市、泳池。'},licences:[{en:'Food Business Licence',zh:'食物業牌照'},{en:'Restricted Food Permit',zh:'限制售賣食物許可證'},{en:'Liquor Licence',zh:'酒牌'}],portal:{label:{en:'Licensing & permits',zh:'牌照及許可證'},url:'https://www.fehd.gov.hk/english/licensing/index.html'},code:'FEHD'},
      {dept:'CR',deptUrl:'https://www.cr.gov.hk/',desc:{en:'Company incorporation & public registers.',zh:'公司註冊及公開登記冊。'},licences:[{en:'Certificate of Incorporation',zh:'公司註冊證書'},{en:"Money Lender's Licence",zh:'放債人牌照'},{en:'TCSP Licence',zh:'信託或公司服務提供者牌照'}]},
      {dept:'IRD',deptUrl:'https://www.gov.hk/en/business/registration/license/index.htm',desc:{en:'Business Registration + tax.',zh:'商業登記 + 稅務。'},licences:[{en:'Business Registration Certificate',zh:'商業登記證'},{en:'Profits tax filing',zh:'利得稅申報'}],portal:{label:{en:'Business Registration',zh:'商業登記'},url:'https://www.gov.hk/en/business/registration/license/index.htm'}},
      {dept:'OFCA',deptUrl:'https://www.ofca.gov.hk/',desc:{en:'Telecommunications & broadcasting licences.',zh:'電訊及廣播牌照。'},licences:[{en:'Unified Carrier Licence (UCL)',zh:'綜合傳送者牌照'},{en:'Broadcasting service licence',zh:'廣播服務牌照'},{en:'Class / radio apparatus licence',zh:'類別／無線電器具牌照'}],portal:{label:{en:'Regulations & licensing',zh:'規管與發牌'},url:'https://www.ofca.gov.hk/en/industry_focus/regulations_licensing/index.html'},code:'OFCA'},
      {dept:'C&ED',deptUrl:'https://www.customs.gov.hk/',desc:{en:'Dutiable commodities, import/export, controlled goods.',zh:'應課稅品、進出口、受管制貨品。'},licences:[{en:'Dutiable Commodities Licence',zh:'應課稅品牌照'},{en:'Liquor / Tobacco permits',zh:'酒類／煙草許可證'},{en:'Import & Export Licence',zh:'進出口牌照'}],portal:{label:{en:'About licences',zh:'關於牌照'},url:'https://www.customs.gov.hk/en/service-enforcement-information/trade-facilitation/dutiable-commodities/about-licences/index.html'}},
      {dept:'EMSD',deptUrl:'https://www.emsd.gov.hk/',desc:{en:'Electrical worker / contractor, gas, lifts.',zh:'電業工程人員/承辦商、氣體、升降機。'},licences:[{en:'Registered Electrical Worker',zh:'註冊電業工程人員'},{en:'Registered Electrical Contractor',zh:'註冊電業承辦商'},{en:'Registered Gas Installer',zh:'註冊氣體裝置技工'}],portal:{label:{en:'How to apply',zh:'申請方法'},url:'https://www.emsd.gov.hk/m/en/electricity_safety/how_to_apply/'}},
      {dept:'LD',deptUrl:'https://www.labour.gov.hk/',desc:{en:'Employment agencies & staffing.',zh:'職業介紹所及人手編制。'},licences:[{en:'Employment Agency Licence',zh:'職業介紹所牌照'}]},
      {dept:'TD',deptUrl:'https://www.td.gov.hk/',desc:{en:'Vehicle, driving & operator licensing.',zh:'車輛、駕駛及營辦商發牌。'},licences:[{en:'Vehicle Registration & Licence',zh:'車輛登記及牌照'},{en:'Driving Licence',zh:'駕駛執照'},{en:'Public / Hire Car permit',zh:'公共／出租汽車許可證'}],code:'TD'},
      {dept:'TIA',deptUrl:'https://www.tia.gov.hk/',desc:{en:'Travel agents, tourist guides (post-TIRO reform).',zh:'旅行代理商、導遊（TIRO 改革後）。'},licences:[{en:'Travel Agent Licence',zh:'旅行代理商牌照'},{en:'Tourist Guide Licence',zh:'導遊牌照'}],code:'TIA'}
    ];
    // ============ funding & credits directory (built on the v10 scaffold) ============
    // Bilingual curated directory. `type` matches a .fc-type-* class + fc_type_*
    // i18n key; `cats` is a subset of [sme,tech,retail,trade,creative] and maps to
    // the fc_cat_* i18n keys used by both the filter chips and the wizard. Every
    // amount/figure is a bilingual string (no machine conversion of 萬). Only
    // currently-open programmes are listed — closed schemes (e.g. Technology
    // Voucher, SFGS 90% Guarantee) are deliberately omitted. Verify on the
    // official page before applying.
    const FUND_PROGRAMMES=[
      // ---- grants ----
      {id:'bud',type:'grant',cats:['sme','retail','trade','creative'],url:'https://www.tid.gov.hk/en/our_work/support_for_trade_industry/bud.html',
        name:{en:'BUD Fund — Branding, Upgrading & Domestic Sales',zh:'BUD 專項基金 — 發展品牌、升級轉型及拓展內銷'},
        provider:{en:'Trade & Industry Dept (TID)',zh:'工業貿易署'},
        amount:{en:'HK$800K per app · HK$7M cumulative per enterprise',zh:'每宗申請 80 萬港元 · 每企業累計最高 700 萬港元'},
        summary:{en:'50:50 matching grant for branding, upgrading and domestic/overseas sales projects. Scope expanded to cover more economies from 15 Jun 2026.',zh:'50:50 配對資助，用於品牌發展、升級轉型及內銷／海外推廣項目。2026 年 6 月 15 日起擴大涵蓋經濟體。'}},
      {id:'emf',type:'grant',cats:['sme','trade','retail'],url:'https://www.tid.gov.hk/en/our_work/support_for_trade_industry/emf.html',
        name:{en:'SME Export Marketing Fund (EMF)',zh:'中小企市場推廣基金（EMF）'},
        provider:{en:'Trade & Industry Dept (TID)',zh:'工業貿易署'},
        amount:{en:'HK$100K per app · HK$1M cumulative per enterprise',zh:'每宗申請 10 萬港元 · 每企業累計最高 100 萬港元'},
        summary:{en:'Reimburses up to 50% of export-promotion costs — overseas fairs, online ads, business missions, promotional materials.',zh:'就出口推廣開支資助最高 50% — 海外展覽、網上廣告、商貿考察、宣傳品。'}},
      {id:'rd-rebate',type:'grant',cats:['tech','sme'],url:'https://www.itf.gov.hk/en/funding-programmes/index.html',
        name:{en:'R&D Cash Rebate Scheme',zh:'研究與發展現金回贈計劃'},
        provider:{en:'Innovation & Technology Commission (ITC)',zh:'創新科技署'},
        amount:{en:'40% cash rebate on eligible R&D',zh:'合資格研發開支 40% 現金回贈'},
        summary:{en:'40% cash rebate on R&D expenditure — applies to company-funded applied R&D and to partnership projects with designated local R&D institutions.',zh:'就研發開支提供 40% 現金回贈 — 適用於公司全資的應用研發及與指定本地研發機構的合作項目。'}},
      {id:'ccmf',type:'grant',cats:['tech'],url:'https://www.cyberport.hk/en/entrepreneurship/cyberport_creative_micro_fund/',
        name:{en:'Cyberport Creative Micro Fund (CCMF)',zh:'數碼港創意微型基金（CCMF）'},
        provider:{en:'Cyberport',zh:'數碼港'},
        amount:{en:'HK$100K seed (6-month project)',zh:'10 萬港元種子資助（6 個月項目）'},
        summary:{en:'Seed funding to turn a digital-tech idea into a proof-of-concept. Hong Kong and international streams available.',zh:'種子資助，將數碼科技構思轉化為概念驗證。設香港及國際組別。'}},
      {id:'fdf',type:'grant',cats:['creative'],url:'https://www.fdc.gov.hk/en/fund/',
        name:{en:'Film Development Fund',zh:'電影發展基金'},
        provider:{en:'Film Development Council',zh:'電影發展局'},
        amount:{en:'Production financing & scriptwriting grants',zh:'電影製作融資及編劇資助'},
        summary:{en:'Financing for film production, scriptwriting incubation and training — nurturing Hong Kong film talent and content.',zh:'資助電影製作、編劇培育及培訓 — 培育香港電影人才及內容。'}},
      {id:'csi',type:'grant',cats:['creative'],url:'https://csi.ccidahk.gov.hk/',
        name:{en:'CreateSmart Initiative (CSI)',zh:'創意智優計劃'},
        provider:{en:'CCIDAHK (CreateHK)',zh:'文創產業發展處（創意香港）'},
        amount:{en:'Project grants (HK$10M+ need LegCo approval)',zh:'項目資助（逾 1,000 萬港元須立法會批核）'},
        summary:{en:'Grants for projects that develop and promote Hong Kong cultural and creative industries — design, film, digital entertainment, publishing and more. Year-round applications.',zh:'資助推動香港文化及創意產業發展與推廣的項目 — 設計、電影、數碼娛樂、出版等。全年接受申請。'}},
      {id:'sie',type:'grant',cats:['creative','sme'],url:'https://www.sie.gov.hk/en/',
        name:{en:'SIE Fund (Social Innovation)',zh:'社會創新及創業發展基金（社創基金）'},
        provider:{en:'Digital Policy Office',zh:'數字政策辦公室'},
        amount:{en:'HK$500M fund pool',zh:'5 億港元基金總額'},
        summary:{en:'Funds innovative projects and social enterprises that alleviate poverty and address social challenges via cross-sector collaboration. Programmes run by intermediaries.',zh:'資助紓緩貧窮、應對社會挑戰的創新項目及社會企業，促進跨界別協作。由中介機構推行各項目。'}},
      {id:'gba-youth',type:'grant',cats:['tech','creative','trade'],url:'https://www.weventure.gov.hk/en/plan_details/index.html',
        name:{en:'GBA Youth Entrepreneurship (Youth Development Fund)',zh:'粵港澳大灣區青年創業資助計劃（青年發展基金）'},
        provider:{en:'Home & Youth Affairs Bureau',zh:'民政及青年事務局'},
        amount:{en:'up to HK$600K seed (ages 18–39)',zh:'最高 60 萬港元種子資助（18–39 歲）'},
        summary:{en:'Seed capital plus 3-year incubation for HK permanent residents aged 18–39 starting up in the GBA. Delivered via funded NGOs with mentorship and dual-base support.',zh:'為 18–39 歲香港永久性居民在大灣區創業提供種子資金及 3 年培育。由獲資助非政府機構提供導師及雙創基地支援。'}},
      {id:'citf',type:'grant',cats:['sme'],url:'https://www.citf.cic.hk/',
        name:{en:'Construction Innovation & Technology Fund (CITF)',zh:'建造業創新及科技基金（CITF）'},
        provider:{en:'Construction Industry Council (DEVB)',zh:'建造業議會（發展局）'},
        amount:{en:'up to HK$2.5M per applicant (AI/robotics)',zh:'每申請者最高 250 萬港元（人工智能／機械人）'},
        summary:{en:'Co-funds adoption of BIM, Modular Integrated Construction, advanced and AI/robotics technologies in construction. Open year-round; fresh HK$1.4B injection in 2026–27.',zh:'共同資助建造業採用 BIM、組裝合成建築、先進及人工智能／機械人技術。全年接受申請；2026–27 年度再注資 14 億港元。'}},
      {id:'pass',type:'grant',cats:['sme','trade'],url:'https://www.pass.gov.hk/main/en/home',
        name:{en:'Professional Services Advancement Support Scheme (PASS)',zh:'專業服務協進計劃（PASS）'},
        provider:{en:'Commerce & Economic Development Bureau',zh:'商務及經濟發展局'},
        amount:{en:'up to HK$3M (90% of project cost)',zh:'最高 300 萬港元（項目成本 90%）'},
        summary:{en:'Funds industry-led, non-profit projects that enhance Hong Kong professional services and external exchanges — legal, accounting, engineering, surveying and more.',zh:'資助業界主導的非牟利項目，提升香港專業服務及對外交流 — 法律、會計、工程、測量等。'}},
      // ---- loan guarantee ----
      {id:'sfgs',type:'loan',cats:['sme','retail','trade','creative'],url:'https://www.hkmc.com.hk/eng/our_business/sme_financing_guarantee_scheme.html',
        name:{en:'SME Financing Guarantee Scheme — 80% Guarantee',zh:'中小企融資擔保計劃 — 八成信貸擔保'},
        provider:{en:'HKMC Insurance (HKMA-backed)',zh:'按揭證券公司保險（金管局支持）'},
        amount:{en:'up to HK$18M · up to 10 years · open to Mar 2028',zh:'最高 1,800 萬港元 · 最長 10 年 · 開放至 2028 年 3 月'},
        summary:{en:'Government guarantees 80% of an SME loan from participating lenders, unlocking financing otherwise hard to secure. Principal moratorium available.',zh:'政府為參與貸款機構的中小企貸款提供 80% 擔保，協助取得原本難以獲得的融資。設還息不還本安排。'}},
      // ---- incubation ----
      {id:'cyberport-inc',type:'incubation',cats:['tech'],url:'https://www.cyberport.hk/en/entrepreneurship/cyberport_incubation_programme/',
        name:{en:'Cyberport Incubation Programme',zh:'數碼港創意培育計劃'},
        provider:{en:'Cyberport',zh:'數碼港'},
        amount:{en:'HK$500K funding + HK$200K rental subsidy (24 mo)',zh:'50 萬港元資助 + 20 萬港元租金補貼（24 個月）'},
        summary:{en:'24-month support for early-stage digital-tech startups scaling up: funding, workspace, mentorship and the Cyberport ecosystem.',zh:'為早期數碼科技初創提供 24 個月擴展支援：資助、工作空間、導師及數碼港生態圈。'}},
      {id:'hkstp-inc',type:'incubation',cats:['tech'],url:'https://www.hkstp.org/en/programmes/incubation/incubation-programme',
        name:{en:'HKSTP Incubation Programme',zh:'科技園公司科技創業培育計劃'},
        provider:{en:'Hong Kong Science & Technology Parks (HKSTP)',zh:'香港科技園公司'},
        amount:{en:'up to HK$1.29M (3 years)',zh:'最高 129 萬港元（3 年）'},
        summary:{en:'3-year deep-tech incubation — funding, R&D labs, subsidised workspace and an investor network for HK tech-based companies.',zh:'3 年深科技培育 — 為香港科技公司提供資助、研發實驗室、租金津貼及投資者網絡。'}},
      {id:'hkstp-ideation',type:'incubation',cats:['tech'],url:'https://www.hkstp.org/en/programmes/ideation',
        name:{en:'HKSTP Ideation Programme',zh:'科技園公司 Ideation 計劃'},
        provider:{en:'Hong Kong Science & Technology Parks (HKSTP)',zh:'香港科技園公司'},
        amount:{en:'HK$100K equity-free seed',zh:'10 萬港元免股權種子資助'},
        summary:{en:'Equity-free seed funding plus mentorship to take a tech idea from concept toward a minimum viable product.',zh:'免股權種子資助配以導師指導，將科技構思由概念推進至最小可行產品。'}},
      {id:'dip',type:'incubation',cats:['creative'],url:'https://www.hkdesignincubation.org/',
        name:{en:'Design Incubation Programme (DIP)',zh:'設計創業培育計劃（DIP）'},
        provider:{en:'Hong Kong Design Centre (CreateHK)',zh:'香港設計中心（創意香港）'},
        amount:{en:'2-year support + mentorship',zh:'2 年支援 + 導師指導'},
        summary:{en:'2-year programme nurturing design startups — financial support, shared studio, mentorship and business networking.',zh:'2 年計劃培育設計初創 — 財務支援、共享工作室、導師指導及商業網絡。'}},
      // ---- investment ----
      {id:'jumpstarter',type:'investment',cats:['tech','creative'],url:'https://www.ent-fund.org/en/',
        name:{en:'JUMPSTARTER',zh:'JUMPSTARTER 創業比賽'},
        provider:{en:'Alibaba Hong Kong Entrepreneurs Fund',zh:'阿里巴巴香港創業者基金'},
        amount:{en:'up to US$5M investment',zh:'最高 500 萬美元投資'},
        summary:{en:'Startup pitch competition — selected founders compete for up to US$5M investment plus access to the Alibaba ecosystem and mentorship.',zh:'初創企業比賽 — 入選創辦人競逐最高 500 萬美元投資，並接入阿里巴巴生態圈及導師網絡。'}},
      {id:'itvf',type:'investment',cats:['tech'],url:'https://www.itf.gov.hk/en/funding-programmes/index.html',
        name:{en:'Innovation & Technology Venture Fund (ITVF)',zh:'創科創投基金（ITVF）'},
        provider:{en:'Innovation & Technology Commission (ITC)',zh:'創新科技署'},
        amount:{en:'Government co-invests with private VCs',zh:'政府與私人創投共同投資'},
        summary:{en:'Government co-invests alongside private venture capital in local I&T startups, mobilising private capital into early-stage tech.',zh:'政府聯同私人創業投資資金共同投資本地創科初創，帶動私人資本投入早期科技項目。'}},
      {id:'cmf',type:'investment',cats:['tech'],url:'https://www.cyberport.hk/en/entrepreneurship/cyberport_macro_fund/',
        name:{en:'Cyberport Macro Fund (CMF)',zh:'數碼港投資創業基金（CMF）'},
        provider:{en:'Cyberport',zh:'數碼港'},
        amount:{en:'HK$1M–20M co-investment (seed to Series A+)',zh:'100 萬至 2,000 萬港元共同投資（種子至 A 輪或以後）'},
        summary:{en:'HK$400M co-investment fund backing Cyberport digital-tech ventures alongside private investors — growth acceleration from seed through Series A and beyond.',zh:'4 億港元共同投資基金，聯同私人投資者投資數碼港數碼科技企業 — 由種子至 A 輪及以後的增長加速。'}},
      // ---- advisory ----
      {id:'startmeup',type:'advisory',cats:['tech','trade','creative'],url:'https://www.startmeup.hk/',
        name:{en:'StartmeupHK',zh:'StartmeupHK'},
        provider:{en:'InvestHK',zh:'投資推廣署'},
        amount:{en:'Free advisory + visa support',zh:'免費顧問 + 簽證支援'},
        summary:{en:'One-stop support for overseas/PRC founders setting up in Hong Kong — advisory, talent and entry visas, network and events.',zh:'為海外／內地創辦人來港發展提供一站式支援 — 顧問、人才及入境簽證、網絡及活動。'}},
      {id:'success',type:'advisory',cats:['sme','retail','trade','creative'],url:'https://www.success.tid.gov.hk/',
        name:{en:'SUCCESS',zh:'中小企一站通（SUCCESS）'},
        provider:{en:'Trade & Industry Dept (TID)',zh:'工業貿易署'},
        amount:{en:'Free SME business consultation',zh:'免費中小企營商諮詢'},
        summary:{en:'Free business advisory for SMEs and startups — consultation, information on government funding and licences, and mentorship.',zh:'為中小企及初創提供免費營商諮詢 — 顧問、政府資助及牌照資訊、導師指導。'}},
      // ---- cloud credits ----
      {id:'gcp',type:'cloud',cats:['tech'],url:'https://cloud.google.com/startup',
        name:{en:'Google for Startups Cloud Program',zh:'Google 新創雲端計劃'},
        provider:{en:'Google Cloud',zh:'Google Cloud'},
        amount:{en:'up to US$200K credits (US$350K AI-first)',zh:'最高 20 萬美元（AI 優先 35 萬美元）'},
        summary:{en:'Up to US$200K in Google Cloud credits over 2 years, plus training and cost-optimisation support. AI-first startups qualify for up to US$350K.',zh:'兩年最高 20 萬美元 Google Cloud 額度，另加培訓及成本優化支援。AI 優先初創最高可獲 35 萬美元。'}},
      {id:'aws',type:'cloud',cats:['tech'],url:'https://aws.amazon.com/startups/credits/',
        name:{en:'AWS Activate',zh:'AWS Activate'},
        provider:{en:'Amazon Web Services',zh:'Amazon Web Services'},
        amount:{en:'US$1K–200K credits',zh:'1,000 至 20 萬美元額度'},
        summary:{en:'Cloud credits, architecture support and training for early-stage startups. Packages scale with stage and backing.',zh:'為早期初創提供雲端額度、架構支援及培訓。額度隨發展階段及背景遞增。'}},
      {id:'ms',type:'cloud',cats:['tech'],url:'https://www.microsoft.com/en-us/startups',
        name:{en:'Microsoft for Startups Founders Hub',zh:'Microsoft 新創服務 Founders Hub'},
        provider:{en:'Microsoft',zh:'Microsoft'},
        amount:{en:'up to US$150K Azure + US$25K Stripe',zh:'最高 15 萬美元 Azure + 2.5 萬美元 Stripe'},
        summary:{en:'Up to US$150K Azure credits (incl. OpenAI), unlocked as the startup hits milestones, plus GitHub Enterprise and partner offers.',zh:'最高 15 萬美元 Azure 額度（包括 OpenAI），隨初創達成里程碑逐步解鎖，另加 GitHub Enterprise 及合作夥伴優惠。'}},
      // ---- ad credits ----
      {id:'ad-grants',type:'ads',cats:['creative'],url:'https://www.google.com/grants/',
        name:{en:'Google Ad Grants',zh:'Google 廣告公益補助'},
        provider:{en:'Google for Nonprofits',zh:'Google 非營利計劃'},
        amount:{en:'US$10K/mo free Search ads',zh:'每月 1 萬美元免費搜尋廣告'},
        summary:{en:'Up to US$10K/month of free Google Search advertising for eligible nonprofits and charities — reach people searching for your cause.',zh:'為合資格非牟利機構及慈善團體提供每月最高 1 萬美元免費 Google 搜尋廣告 — 觸及搜尋您理念的人。'}}
    ];
    // "I am…" wizard situations. Each `id` doubles as the business category it
    // maps to (so the wizard and the filter chips share one taxonomy). `picks`
    // lists programme ids in priority order by stage.
    const FUND_SCENARIOS=[
      {id:'sme',blurb:{en:'Operating an SME? Stack the matching grants + a guaranteed loan first, then free advisory.',zh:'營運中的中小企？先疊加配對資助 + 信貸擔保貸款，再配以免費諮詢。'},
        picks:['bud','emf','sfgs','success','rd-rebate','pass','citf']},
      {id:'tech',blurb:{en:'Tech startup? Move seed, then incubation, then R&D rebate, and layer vendor cloud credits throughout.',zh:'科技初創？按種子、培育、研發回贈推進，並全程疊加供應商雲端額度。'},
        picks:['hkstp-ideation','ccmf','gba-youth','hkstp-inc','cyberport-inc','rd-rebate','gcp','aws','ms','jumpstarter','itvf','cmf']},
      {id:'retail',blurb:{en:'Retail or F&B? Brand/upgrade grants and export marketing fund cover expansion; the guarantee scheme funds fit-out.',zh:'零售或餐飲？品牌升級資助及市場推廣基金涵蓋擴展；信貸擔保計劃資助裝修。'},
        picks:['bud','emf','sfgs','success']},
      {id:'trade',blurb:{en:'Trading company? The Export Marketing Fund and BUD Fund are your core engines overseas.',zh:'貿易公司？市場推廣基金及 BUD 專項基金是您拓展海外的核心引擎。'},
        picks:['emf','bud','sfgs','success','startmeup','pass']},
      {id:'creative',blurb:{en:'Creative industries? Design and film incubation first, then grants and free ad reach for mission-driven work.',zh:'創意產業？先參與設計及電影培育，再申請資助，並為理念導向工作取得免費廣告觸及。'},
        picks:['dip','fdf','csi','sie','ccmf','gba-youth','bud','sfgs','ad-grants','startmeup']}
    ];
    let MARKET_PLAYERS={}, marketPlayersLoaded=false;
    async function loadMarketPlayers(){ if(marketPlayersLoaded) return; marketPlayersLoaded=true; const groups=await getJSON('/v1/market-players'); if(groups&&!groups.__error&&Array.isArray(groups)){ for(const g of groups){ if(g&&g.dept&&Array.isArray(g.players)) MARKET_PLAYERS[g.dept.toUpperCase()]=g; } } }
    function playersPanel(code){ if(!code) return ''; const g=MARKET_PLAYERS[(code||'').toUpperCase()]; if(!g||!g.players||!g.players.length) return ''; const items=g.players.slice(0,10).map((p,i)=>`<div class="player"><div class="rank">${i+1}</div><div><span class="nm">${escapeHtml(p.name)}</span> <span class="nt">— ${escapeHtml(p.note||'')}</span></div></div>`).join(''); return `<div class="players"><div class="players-hd"><i class="ri-building-line"></i> ${t('lic_players')} (${g.category||'HK market'})</div>${items}<div class="players-disc">${t('lic_players_disc')}</div></div>`; }
    function runWizard(){ const sel=document.getElementById('wizSelect'); const sc=LIC_SCENARIOS.find(s=>s.id===sel.value); const box=document.getElementById('wizResult'); if(!sc){ box.innerHTML='<div class="empty">'+t('lic_choose')+'</div>'; return; } box.innerHTML=`<div style="font-weight:600;margin-bottom:8px">${escapeHtml(L(sc.label))}</div>`+sc.steps.map((s,i)=>`<div class="wiz-step"><div class="num">${i+1}</div><div style="flex:1"><b>${escapeHtml(L(s.lic))}</b> <span style="color:var(--muted)">· ${escapeHtml(L(s.who))} · ${escapeHtml(L(s.when))}</span><div style="margin-top:4px">${extBtn(s.url,t('lic_official'))}</div></div></div>`).join(''); }
    function renderLicences(){ const sel=document.getElementById('wizSelect'); if(sel&&sel.options.length<=1){ for(const s of LIC_SCENARIOS) sel.options.add(new Option(L(s.label),s.id)); } document.getElementById('licencesKey').innerHTML=LIC_KEY.map(k=>`<div class="lic-key"><div class="kt">${escapeHtml(L(k.t))}</div><div class="kd">${escapeHtml(L(k.d))}</div>${extBtn(k.u,t('lic_open_portal'))}</div>`).join(''); document.getElementById('licencesHighlights').innerHTML=LIC_HIGHLIGHTS.map(h=>`<div class="lic-hl"><div class="name">${escapeHtml(L(h.name))}</div><div class="who">${escapeHtml(L(h.dept))}</div>${extBtn(h.url,t('lic_how_apply'))}</div>`).join(''); const q=(document.getElementById('licSearch').value||'').trim().toLowerCase(); const rows=LIC_DEPARTMENTS.filter(d=>{ if(!q) return true; return (deptName(d.dept)+' '+L(d.desc)+' '+d.licences.map(l=>L(l)).join(' ')).toLowerCase().includes(q); }); const cards=rows.map(d=>{ const lics=d.licences.map(l=>`<div>${escapeHtml(L(l))}</div>`).join(''); const portal=d.portal?`<div class="portal">${extBtn(d.portal.url,L(d.portal.label))}</div>`:''; const players=playersPanel(d.code); const dept=`<a class="ext-link" href="${escapeHtml(d.deptUrl)}" target="_blank" rel="noopener noreferrer" data-action="open-ext" data-url="${escapeHtml(d.deptUrl)}"><i class="ri-arrow-right-up-line"></i> ${escapeHtml(deptName(d.dept))}</a>`; return `<div class="lic-card"><div class="dept">${dept}</div><div class="desc">${escapeHtml(L(d.desc))}</div><div class="lics"><b>${t('lic_licences_label')}</b> ${lics}</div>${portal}${players}</div>`; }).join(''); document.getElementById('licencesDir').innerHTML=cards||'<div class="empty">'+t('lic_no_match')+'</div>'; document.getElementById('licencesNote').innerHTML=`<i class="ri-information-line"></i> ${t('lic_note')} ${extBtn(LIC_KEY[0].u,t('lic_search_blis'))}.`; }
    // ============ funding & credits render (built on the v10 scaffold) ============
    // FUND_CAT_FILTER is the active business-category chip ('' = show all).
    // Catalogue of one-off wizard fills so toggleLang() can reset + retranslate.
    let fundCatFilter='';
    const FUND_TYPES=['grant','loan','incubation','investment','advisory','cloud','ads'];
    function fundTypeBadge(type){ return `<span class="fc-type fc-type-${type}">${escapeHtml(t('fc_type_'+type))}</span>`; }
    function fundCatLabel(cat){ return t('fc_cat_'+cat); }
    function fundCard(p, exact){ // exact=true means this programme matches the active filter (green border)
      const cats=p.cats.map(c=>`<span class="cat ${(fundCatFilter===c)?'match':''}">${escapeHtml(fundCatLabel(c))}</span>`).join('');
      return `<div class="fc-card ${exact?'fc-exact':''}"><div class="fc-head">${fundTypeBadge(p.type)}<span class="fc-name">${escapeHtml(L(p.name))}</span></div><div class="fc-provider">${escapeHtml(L(p.provider))}</div><div class="fc-amount">${escapeHtml(L(p.amount))}</div><div class="fc-summary">${escapeHtml(L(p.summary))}</div><div class="fc-cats">${cats}</div><div class="fc-link">${extBtn(p.url,t('fc_official'))}</div></div>`;
    }
    function renderFund(){
      const wizSel=document.getElementById('fundWizSelect');
      if(wizSel && wizSel.options.length<=1){ for(const s of FUND_SCENARIOS) wizSel.options.add(new Option(t('fc_cat_'+s.id), s.id)); }
      // legend
      document.getElementById('fundLegend').innerHTML='<span class="lk" style="font-weight:600">'+escapeHtml(t('fc_legend'))+'</span>'+FUND_TYPES.map(ty=>`<span class="lk">${fundTypeBadge(ty)}</span>`).join('');
      // filter chips
      const chipBtns=['all','sme','tech','retail','trade','creative'].map(c=>{ const active=(fundCatFilter===''&&c==='all')||(fundCatFilter===c); return `<button class="${active?'active':''}" data-action="set-fund-filter" data-cat="${c==='all'?'':c}">${escapeHtml(t('fc_cat_'+c))}</button>`; }).join('');
      document.getElementById('fundChips').innerHTML=`<span style="font-size:11px;color:var(--muted)">${escapeHtml(t('fc_filter_by'))}</span>${chipBtns}<button class="fc-clear" data-action="set-fund-filter" data-cat="">${escapeHtml(t('fc_clear'))}</button>`;
      // directory grid (filtered by active category)
      const rows = fundCatFilter ? FUND_PROGRAMMES.filter(p=>p.cats.includes(fundCatFilter)) : FUND_PROGRAMMES;
      const cards = rows.map(p=>fundCard(p, !!fundCatFilter)).join('');
      document.getElementById('fundDir').innerHTML = cards || '<div class="empty">'+escapeHtml(t('fc_no_match'))+'</div>';
      // count line above grid
      const countNode=document.getElementById('fundDir').previousElementSibling;
      if(rows.length<FUND_PROGRAMMES.length){ const cnt=document.createElement('div'); cnt.id='fundCount'; cnt.style.cssText='font-size:12px;color:var(--muted);margin:-4px 0 10px;'; cnt.textContent=t('fc_count',{n:rows.length,t:FUND_PROGRAMMES.length}); const old=document.getElementById('fundCount'); if(old){old.replaceWith(cnt);}else{countNode.parentNode.insertBefore(cnt,countNode.nextSibling);} }
      else { document.getElementById('fundCount')?.remove(); }
      document.getElementById('fundNote').innerHTML=`<i class="ri-information-line"></i> ${escapeHtml(t('fc_note'))}`;
    }
    function setFundFilter(cat){ fundCatFilter=cat||''; renderFund(); }
    function runFundWizard(){ const sel=document.getElementById('fundWizSelect'); const sc=FUND_SCENARIOS.find(s=>s.id===sel.value); const box=document.getElementById('fundWizResult'); if(!sc){ box.innerHTML='<div class="empty">'+escapeHtml(t('fc_choose'))+'</div>'; return; } const progs=sc.picks.map(id=>FUND_PROGRAMMES.find(p=>p.id===id)).filter(Boolean); box.innerHTML=`<div style="font-weight:600;margin-bottom:4px">${escapeHtml(t('fc_cat_'+sc.id))}</div><div style="color:var(--muted);font-size:12px;margin-bottom:10px">${escapeHtml(L(sc.blurb))}</div>`+progs.map((p,i)=>`<div class="wiz-step"><div class="num">${i+1}</div><div style="flex:1"><b>${escapeHtml(L(p.name))}</b> <span style="color:var(--accent);font-size:12px">· ${escapeHtml(L(p.amount))}</span><div style="margin-top:4px;color:var(--muted);font-size:12px">${escapeHtml(L(p.summary))}</div><div style="margin-top:4px">${extBtn(p.url,t('fc_official'))}</div></div></div>`).join(''); }


