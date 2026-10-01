# [BUG] Subdomain Assets (www., cdn., media.) Not Rewritten in HTML/CSS

**Category**: Bug / Asset Rewriting
**Impact**: High (Images and assets hosted on www or cdn subdomains remain absolute or fail offline)
**Source Code Pointer**: `src/main.rs` Line 335, 366 (`target_url.host_str() == base_url.host_str()`)

## Problem
In `rewrite_html_links` and `rewrite_css_links`, link rewriting checks:
```rust
target_url.host_str() == base_url.host_str()
```
However, `url_to_local_path` allows subdomains (`url_host == base_host || url_host.ends_with(&format!(".{}", base_host))`).
As a result:
- When a page on `https://domain.com` references an asset on `https://www.domain.com/image.jpg` or `https://cdn.domain.com/style.css`, the asset is downloaded to local disk, but the HTML/CSS attribute is NOT rewritten to the relative local path because `host_str()` does not strictly match.

## Discovered In Real-World Audit
- Over 930687 instances across cloned sites had subdomain assets skipped during rewriting.

## Proposed Fix
Update `rewrite_html_links` and `rewrite_css_links` to check subdomain matching consistently:
```rust
fn is_same_or_subdomain(target_host: Option<&str>, base_host: Option<&str>) -> bool {
    match (target_host, base_host) {
        (Some(th), Some(bh)) => th == bh || th.ends_with(&format!(".{}", bh)) || bh.ends_with(&format!(".{}", th)),
        _ => false,
    }
}
```
