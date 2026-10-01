# [BUG] Protocol-Relative URLs (`//cdn.domain.com/...`) Fail Host Matching

**Category**: Bug / Edge Case
**Impact**: Medium (Scripts or fonts using `//domain.com` or `//cdn.domain.com` are treated as external or fail joining)
**Source Code Pointer**: `src/main.rs` Line 254 (`page_url.join(val)`)

## Problem
Protocol-relative URLs like `//example.com/assets/app.js` should inherit the scheme of the `page_url`. While `Url::join` resolves them to the current scheme, subsequent host validation sometimes misclassifies them when stripped or rewritten in `srcset`.

## Proposed Fix
Ensure protocol-relative URLs in `srcset` and asset selectors are normalized prior to path resolution.
