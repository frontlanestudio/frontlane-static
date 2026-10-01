# [FEATURE] Configurable Sitemap Discovery Timeout & Graceful Spider Fallback

**Category**: Feature / Performance & Robustness
**Impact**: High for Enterprise Law Firm Sites with 10k+ Post Sitemaps
**Source Code Pointer**: `src/main.rs` Line 1148 (`website.crawl_sitemap().await`)

## Problem
Certain competitor sites have massive sitemaps (thousands of blog posts and location tags) or slow XML responses that block the initial crawl phase.

## Proposed Feature
1. Add `--sitemap-timeout <SECONDS>` (default 30s) to limit sitemap discovery.
2. If sitemap discovery exceeds the threshold, log a warning and seamlessly fall back to HTML link spidering without aborting the entire backup.
