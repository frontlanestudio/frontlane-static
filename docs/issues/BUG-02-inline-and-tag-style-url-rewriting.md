# [BUG] Inline `style="..."` and `<style>` Tag `url()` Assets Not Extracted or Rewritten

**Category**: Bug / Missing Feature
**Impact**: Medium-High (Hero background images, custom icons, and typography embedded in WordPress inline CSS fail offline)
**Source Code Pointer**: `src/main.rs` Line 225 (`extract_assets`) and Line 298 (`rewrite_html_links`)

## Problem
- `extract_assets` only queries specific HTML attributes: `img[src]`, `script[src]`, `link[href]`, etc.
- It does NOT parse `style` attributes (e.g., `<div style="background-image: url('...');">`) or `<style>` blocks inside HTML pages.
- Similarly, `rewrite_html_links` only matches `href|src|action|data|poster|srcset`. It never runs `rewrite_css_links` on the text within `<style>` blocks or `style="..."` attributes.

## Discovered In Real-World Audit
- Found 7706 instances of unrewritten inline background images across cloned sites.

## Proposed Fix
1. In `extract_assets`, query `[style]` elements and `<style>` tags, extract all `url(...)` targets, and add them to the download queue.
2. In `rewrite_html_links`, parse `style="..."` attributes and `<style>...</style>` contents with `rewrite_css_links` so relative paths are cleanly inserted.
