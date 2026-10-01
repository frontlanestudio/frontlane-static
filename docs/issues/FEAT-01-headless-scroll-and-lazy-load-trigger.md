# [FEATURE] Headless Mode: Dynamic Scroll-to-Bottom & Lazy-Load Triggering

**Category**: Feature / Headless Crawler
**Impact**: High for Modern SPAs and Lazy-Loaded Media
**Source Code Pointer**: `src/main.rs` Line 1266 (`browser.new_tab()`)

## Problem
When crawling dynamic websites with `--headless`, pages often use IntersectionObserver to lazy-load images and footer content. If the headless tab only waits for navigation, lazy-loaded images (like `data-src` / `loading="lazy"`) are not triggered in the DOM and never get downloaded.

## Proposed Feature
Add a configurable `--headless-scroll` option that scrolls the page incrementally to bottom and waits for network idle before extracting DOM content.
