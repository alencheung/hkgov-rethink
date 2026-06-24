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
