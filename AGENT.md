# AGENT.md — agent working agreement for hkgov-rethink

> Rules every contributor (human or AI agent) must follow when working in this
> repository. Read this before touching UI or user-visible strings.

## Iconography: NO emoji. Use Remix Icon.

**Never use emoji in the UI or codebase.** This applies to:

- HTML / templates / rendered strings
- JavaScript-generated markup and button labels
- Logos, badges, severity indicators, status dots
- Markdown that ships as product surface (e.g. `dashboard/`, rendered docs)
- Code comments and doc-strings *only* when they would render to the user

**Use [Remix Icon](https://remixicon.com/) instead.** It is open-source
(Apache-2.0), neutral-styled, and consistent across platforms (emoji render
differently on every OS and break the visual language).

### How to use it

1. **Load the stylesheet** once per page (in `<head>`):
   ```html
   <link href="https://cdn.jsdelivr.net/npm/remixicon@4.2.0/fonts/remixicon.css" rel="stylesheet">
   ```
2. **Render an icon** with an `<i>` tag, class `ri-{name}-{style}`:
   - `-line` = outlined (default for UI chrome)
   - `-fill` = filled (default for status/severity)
   ```html
   <i class="ri-close-line"></i>          <!-- close button -->
   <i class="ri-error-warning-fill"></i>  <!-- severity: warning -->
   ```
3. **Browse names** at https://remixicon.com (search is authoritative).

### Conventions for this project

| Concept | Icon class |
|---|---|
| Severity — critical | `ri-error-warning-fill` (red via CSS) |
| Severity — warning | `ri-alert-line` (amber) |
| Severity — info | `ri-information-line` (blue) |
| Refresh / reload | `ri-refresh-line` |
| Close (modal) | `ri-close-line` |
| Copy to clipboard | `ri-file-copy-line` |
| Download | `ri-download-line` |
| Cite / bookmark | `ri-bookmark-line` |
| Investigate / search | `ri-search-line` |
| History / clock | `ri-time-line` |
| Thumbs up (useful) | `ri-thumb-up-line` |
| Thumbs down (not useful) | `ri-thumb-down-line` |
| Warning banner | `ri-alert-line` |
| Check / success | `ri-check-line` |
| External link | `ri-arrow-right-up-line` |

When in doubt, pick a `-line` variant for UI controls and a `-fill` variant
for status indicators, and confirm the name exists at remixicon.com.

### Why this rule exists

- Emoji render inconsistently across OS/browser (Windows vs macOS vs Linux
  show different glyphs and colors), undermining the dark, neutral design.
- Emoji are not themeable via CSS; Remix Icon inherits `color` and `font-size`,
  so it stays consistent with `--crit`/`--warn`/`--ok` variables.
- Emoji in source strings break screen readers and copy-paste workflows.

### Enforcement

A grep check should be added to CI:
```bash
# Fail if any emoji are present in UI-facing files.
! grep -rP '[\x{1F000}-\x{1FAFF}\x{2600}-\x{27BF}\x{2190}-\x{21FF}\x{2B00}-\x{2BFF}]' dashboard/ crates/api/src/routes.rs
```

## i18n: every user-visible string is translatable, no exceptions

The dashboard is bilingual (English `en` + Traditional Chinese `zh`).
**Every user-visible string must resolve through the i18n system** — never ship
raw English (or raw Chinese) in the DOM or in JS-generated markup. Coverage is
auditable (see Enforcement below), so gaps are detectable, not silent.

### What counts as "user-visible"

This is exhaustive — if a human can read it in the rendered UI, it is in scope:

- Card text, card titles, card subtitles, card descriptions/bodies
- Button labels (incl. icon-only buttons — the label lives in
  `aria-label`/`title`, which **must** also be translated)
- Tooltips (`title=`, custom tooltip elements)
- Placeholders (`<input placeholder="…">`, `<textarea>`)
- Page/section headings: `<h1>`–`<h6>`, header and footer text
- Tab labels, nav items, breadcrumbs
- Empty states, loading states, error messages, toasts, banners
- `<table>` headers (`<th>`), column labels, legend entries
- Form labels (`<label>`), field hints, validation messages
- `aria-label` / `aria-description` and any other accessibility attributes
- Strings built in JS and injected via `innerHTML` / `textContent`
- `<title>` and `<meta name="description">` (lang-aware)

### How to wire a string through i18n

This project uses **three** mechanisms. Pick by where the string lives:

| Where the string is | Mechanism | Example |
|---|---|---|
| Static HTML element's text | `data-i18n="key"` | `<button data-i18n="nav_overview">Overview</button>` |
| Static HTML `<input>`/`<textarea>` placeholder | `data-i18n-ph="key"` | `<input data-i18n-ph="ds_search" placeholder="search datasets…" />` |
| String built in JS (dynamic markup, attributes, toasts) | `t('key', {placeholders})` | `t('div_no_pairs', {n})` |

`applyI18n()` walks `[data-i18n]` (→ `textContent`) and `[data-i18n-ph]`
(→ `placeholder`) once per render from the `I18N` dictionary in
`dashboard/index.html`. `t()` looks up `I18N[curLang]` with `{placeholder}`
substitution for JS-built strings. **English (`I18N.en`) is the source of
truth and the fallback** — `zh` is its mirror.

### The rule, step by step

When you add or change **any** UI string:

1. **Add the key to BOTH `I18N.en` and `I18N.zh`** in `dashboard/index.html`.
   Never add a key to only one language. A key present in `en` but missing in
   `zh` (or vice versa) is a defect — the fallback to `en` is for safety, not
   a license to skip `zh`.
2. **Wire the element** with `data-i18n="key"` (text), `data-i18n-ph="key"`
   (placeholder), or `t('key', …)` (JS). No raw literal reaches the DOM.
3. **Translate, don't transliterate.** Provide a real Traditional Chinese
   (`zh-HK`) rendering. Machine transliteration of English is not acceptable.
4. **Re-run dynamic renderers after `toggleLang()`.** Structured data
   (insights, table rows, licences) is rebuilt by `renderLicences()`,
   `loadSources()`, `renderTimeline()`, etc. If your change adds a new
   translatable surface inside dynamic content, ensure the renderer is called
   in `toggleLang()` so the new language applies immediately.
5. **Keep evidence values language-neutral.** Numbers, dates, dataset IDs,
   and code snippets pass through untranslated — never bake them into a dict
   value; use `{placeholder}` substitution via `t()` instead.

### Anti-patterns (do not ship these)

- ❌ `<button>Save</button>` — raw English, no key. → ✅ `<button data-i18n="btn_save">Save</button>`
- ❌ `title="Close"` — untranslated tooltip. → ✅ translate via a key (this
  project currently has no `data-i18n-title`; if you need one, add the
  `querySelectorAll('[data-i18n-title]')` branch to `applyI18n()` and use it).
- ❌ `el.innerHTML = 'Loading…'` — literal in JS. → ✅ `el.textContent = t('loading')`
- ❌ Adding `btn_save:'Save'` to `I18N.en` only. → ✅ add the matching `zh`
  entry in the same commit.
- ❌ Hard-coding a count: `'Found ' + n + ' rows'`. → ✅ `t('found_rows', {n})`
  with `found_rows:'Found {n} rows'` / `'找到 {n} 筆資料'`.

### Enforcement

Before declaring UI work done, verify coverage with:

```bash
# 1. Every data-i18n / data-i18n-ph key used in markup exists in BOTH languages.
#    Collect keys from the DOM, then confirm each appears under I18N.en AND I18N.zh.
rg -o 'data-i18n(?:-ph)?="([^"]+)"' dashboard/index.html -r '$1' | sort -u

# 2. Symmetric dictionaries: keys present in I18N.en should be present in I18N.zh
#    (and vice versa). A diff here is a missing-translation bug.
#    (Inspect the I18N={en:{…},zh:{…}} block in dashboard/index.html.)

# 3. No raw user-visible English literals leaked outside the I18N dictionary.
#    Spot-check newly added elements for hardcoded text/placeholder/title/aria-label.
rg -n '\splaceholder="[^"]+"' dashboard/index.html   # each should pair with data-i18n-ph
rg -n '\stitle="[^"]+"'    dashboard/index.html      # each should be a translated key
```

CI should fail the build if (a) any `data-i18n*` key lacks an `en` or `zh`
entry, or (b) `I18N.en` and `I18N.zh` key sets differ.
