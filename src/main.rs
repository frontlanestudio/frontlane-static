use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use reqwest::Client;
use scraper::{Html, Selector};
use spider::tokio;
use spider::website::Website;
use std::collections::HashMap;
use dashmap::{DashSet, DashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;
use url::Url;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use colored::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use regex::Regex;
use pathdiff::diff_paths;
use tracing::{info, warn, debug};
use tracing_subscriber::FmtSubscriber;
use std::future::Future;
use std::pin::Pin;

#[derive(ValueEnum, Clone, Debug)]
enum UrlConstraint {
    Strict,
    Host,
}

fn get_styles() -> clap::builder::styling::Styles {
    clap::builder::styling::Styles::styled()
        .header(clap::builder::styling::AnsiColor::Green.on_default() | clap::builder::styling::Effects::BOLD)
        .usage(clap::builder::styling::AnsiColor::Green.on_default() | clap::builder::styling::Effects::BOLD)
        .literal(clap::builder::styling::AnsiColor::Cyan.on_default() | clap::builder::styling::Effects::BOLD)
        .placeholder(clap::builder::styling::AnsiColor::Cyan.on_default())
}

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None, styles = get_styles())]
struct Args {
    /// URL of the website to backup, or 'serve' to preview, or 'audit' to evaluate a directory
    #[arg(default_value = "")]
    url: String,

    /// Directory path when using 'serve' or 'audit' mode (e.g. frontlane-static serve ./backup)
    #[arg(index = 2)]
    serve_dir: Option<String>,

    /// Live site URL to compare against when running standalone audit
    #[arg(long)]
    live_url: Option<String>,

    /// Automatically run clone fidelity and link integrity audit after crawling
    #[arg(long)]
    audit: bool,

    /// Forward offline <form> submissions to a custom webhook URL
    #[arg(long)]
    form_webhook: Option<String>,

    /// Generate an offline client-side search index (search_index.json)
    #[arg(long)]
    generate_search: bool,

    /// Automatically download and localize Google Fonts and typography
    #[arg(long, default_value_t = true)]
    localize_fonts: bool,

    /// Maximum number of social media posts to scrape
    #[arg(long)]
    max_posts: Option<usize>,

    /// Extract structured social data (posts.json, posts.csv)
    #[arg(long)]
    extract_data: bool,

    /// Serve directory with built-in preview server
    #[arg(long)]
    serve: Option<Option<String>>,

    /// Port for the preview server (default: 3000)
    #[arg(long, default_value_t = 3000)]
    port: u16,

    /// Host for the preview server (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Enable SPA / 404 fallback to index.html in preview server
    #[arg(long)]
    spa: bool,

    /// Output directory
    #[arg(short, long)]
    output: Option<String>,

    /// Maximum file size for assets (in bytes)
    #[arg(long, default_value_t = 10_000_000)]
    max_size: u64,
    
    /// Minimum file size for assets (in bytes)
    #[arg(long, default_value_t = 0)]
    min_size: u64,

    /// Ignore robots.txt
    #[arg(long)]
    ignore_robots_txt: bool,

    /// Ignore rel="nofollow"
    #[arg(long)]
    ignore_nofollow: bool,

    /// Download error pages
    #[arg(long)]
    download_error_pages: bool,

    /// Maximum number of levels (depth)
    #[arg(long, default_value_t = 4)]
    max_depth: usize,

    /// Maximum number of files (0 for unlimited)
    #[arg(long, default_value_t = 0)]
    max_files: usize,

    /// Number of concurrent connections
    #[arg(long, default_value_t = 6)]
    connections: usize,

    /// User-Agent string to use
    #[arg(long, default_value = "SiteSucker/1.0")]
    user_agent: String,

    /// Number of attempts (retries)
    #[arg(long, default_value_t = 2)]
    retries: usize,

    /// Request timeout in seconds
    #[arg(long, default_value_t = 60)]
    timeout: u64,

    /// Sitemap discovery timeout in seconds (falls back to HTML spidering on timeout)
    #[arg(long, default_value_t = 30)]
    sitemap_timeout: u64,

    /// Delay between requests in seconds
    #[arg(long, default_value_t = 0.0)]
    delay: f64,

    /// Disable sitemap scanning (sitemaps are scanned by default)
    #[arg(long)]
    no_sitemaps: bool,

    /// URL constraint (strict or host)
    #[arg(long, value_enum, default_value_t = UrlConstraint::Host)]
    constraint: UrlConstraint,

    /// Replace special characters with '_' in filenames
    #[arg(long)]
    replace_special_chars: bool,

    /// Use a headless browser to fetch and render JavaScript-heavy pages
    #[arg(long)]
    headless: bool,

    /// Incrementally scroll pages to bottom in headless mode to trigger lazy-loaded assets
    #[arg(long)]
    headless_scroll: bool,

    /// Export pages to PDF (requires a local headless Chrome installation)
    #[arg(long)]
    export_pdf: bool,

    /// Extract page content as LLM-ready Markdown
    #[arg(long)]
    export_markdown: bool,

    /// Extract page metadata (title, descriptions) to metadata.csv
    #[arg(long)]
    extract_metadata: bool,

    /// Target specific HTML elements using a CSS selector (discards the rest)
    #[arg(long)]
    css_selector: Option<String>,
    
    /// Basic authentication in format username:password
    #[arg(long)]
    auth: Option<String>,

    /// Raw cookie string to send with requests
    #[arg(long)]
    cookie: Option<String>,

    /// Regex patterns to exclude from crawling
    #[arg(long)]
    exclude_regex: Option<Vec<String>>,

    /// Dry run (do not download files)
    #[arg(long)]
    dry_run: bool,
    
    /// Verbose logging
    #[arg(short, long)]
    verbose: bool,
}

fn clean_host(host: &str) -> &str {
    let h = host.split(':').next().unwrap_or(host);
    h.strip_prefix("www.").unwrap_or(h)
}

fn is_same_or_subdomain(target_host: Option<&str>, base_host: Option<&str>) -> bool {
    match (target_host, base_host) {
        (Some(th), Some(bh)) => {
            if th.eq_ignore_ascii_case(bh) {
                return true;
            }
            let th_lower = th.to_ascii_lowercase();
            let bh_lower = bh.to_ascii_lowercase();
            let t_clean = clean_host(&th_lower);
            let b_clean = clean_host(&bh_lower);
            if t_clean == b_clean {
                return true;
            }
            if th_lower == "fonts.googleapis.com" || th_lower == "fonts.gstatic.com" {
                return true;
            }
            if th_lower.len() > bh_lower.len() && th_lower.ends_with(&format!(".{}", bh_lower)) {
                return true;
            }
            if bh_lower.len() > th_lower.len() && bh_lower.ends_with(&format!(".{}", th_lower)) {
                return true;
            }
            if t_clean.len() > b_clean.len() && t_clean.ends_with(&format!(".{}", b_clean)) {
                return true;
            }
            if b_clean.len() > t_clean.len() && b_clean.ends_with(&format!(".{}", t_clean)) {
                return true;
            }
            let th_parts: Vec<&str> = t_clean.split('.').collect();
            let bh_parts: Vec<&str> = b_clean.split('.').collect();
            if th_parts.len() >= 2 && bh_parts.len() >= 2 {
                let th_root = format!("{}.{}", th_parts[th_parts.len() - 2], th_parts[th_parts.len() - 1]);
                let bh_root = format!("{}.{}", bh_parts[bh_parts.len() - 2], bh_parts[bh_parts.len() - 1]);
                if th_root == bh_root {
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

fn sanitize_filename(filename: &str) -> String {
    filename.chars().map(|c| {
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
            c
        } else {
            '_'
        }
    }).collect()
}

fn url_to_local_path(url: &Url, base_url: &Url, output_dir: &Path, replace_special: bool, force_extension: Option<&str>) -> Option<PathBuf> {
    if !is_same_or_subdomain(url.host_str(), base_url.host_str()) {
        return None; 
    }
    
    let path = url.path().trim_start_matches('/');
    let mut local_path = output_dir.to_path_buf();
    
    let query_suffix = url.query().map(|q| {
        let sanitized = sanitize_filename(q);
        format!("_{}", sanitized)
    }).unwrap_or_default();
    
    if path.is_empty() || url.path().ends_with('/') {
        if replace_special {
            let parts: Vec<String> = path.split('/').map(sanitize_filename).collect();
            for part in parts {
                if !part.is_empty() {
                    local_path.push(part);
                }
            }
        } else {
            local_path.push(path);
        }
        local_path.push(format!("index{}", query_suffix));
        if let Some(ext) = force_extension {
            local_path.set_extension(ext);
        } else {
            local_path.set_extension("html");
        }
    } else {
        let mut parts: Vec<&str> = path.split('/').collect();
        let last_part = parts.pop().unwrap_or("");
        
        for part in parts {
            if replace_special {
                local_path.push(sanitize_filename(part));
            } else {
                local_path.push(part);
            }
        }
        
        let filename = if replace_special {
            sanitize_filename(last_part)
        } else {
            last_part.to_string()
        };
        
        let temp_path = PathBuf::from(&filename);
        let orig_ext = temp_path.extension().and_then(|s| s.to_str()).map(|s| s.to_string());
        let stem = temp_path.file_stem().and_then(|s| s.to_str()).unwrap_or(&filename);
        
        if let Some(ext) = force_extension {
            let new_filename = format!("{}{}.{}", stem, query_suffix, ext);
            local_path.push(new_filename);
        } else if let Some(ref ext) = orig_ext {
            let new_filename = format!("{}{}.{}", stem, query_suffix, ext);
            local_path.push(new_filename);
        } else {
            local_path.push(&filename);
            local_path.push(format!("index{}.html", query_suffix));
        }
    }
    Some(local_path)
}

fn extract_assets(html_content: &str, page_url: &Url) -> Vec<Url> {
    let mut assets = Vec::new();
    let document = Html::parse_document(html_content);
    
    let selectors = vec![
        ("a[href]", "href"),
        ("area[href]", "href"),
        ("img[src]", "src"),
        ("script[src]", "src"),
        ("link[href]", "href"),
        ("video[src]", "src"),
        ("video[poster]", "poster"),
        ("audio[src]", "src"),
        ("source[src]", "src"),
        ("embed[src]", "src"),
        ("iframe[src]", "src"),
        ("frame[src]", "src"),
        ("object[data]", "data"),
        ("track[src]", "src"),
        ("form[action]", "action"),
        ("input[type='image'][src]", "src"),
        ("meta[property='og:image'][content]", "content"),
        ("meta[property='og:image:secure_url'][content]", "content"),
        ("meta[name='twitter:image'][content]", "content"),
        ("link[rel='icon'][href]", "href"),
        ("link[rel='shortcut icon'][href]", "href"),
        ("link[rel='apple-touch-icon'][href]", "href"),
        ("link[rel='apple-touch-icon-precomposed'][href]", "href"),
        ("link[rel='manifest'][href]", "href"),
        // Lazy loading & CMS attributes (BUG-04)
        ("img[data-src]", "data-src"),
        ("img[data-lazy-src]", "data-lazy-src"),
        ("img[data-original]", "data-original"),
        ("img[data-orig-file]", "data-orig-file"),
        ("img[data-large_image]", "data-large_image"),
        ("img[data-large-file]", "data-large-file"),
        ("img[data-medium-file]", "data-medium-file"),
        ("img[data-full-url]", "data-full-url"),
        ("img[data-thumb]", "data-thumb"),
        ("img[data-url]", "data-url"),
        ("[data-bg]", "data-bg"),
        ("[data-bg-hidpi]", "data-bg-hidpi"),
        ("[data-background-image]", "data-background-image"),
        // SVG refs
        ("use", "href"),
        ("image", "href"),
    ];

    for (sel_str, attr) in selectors {
        if let Ok(selector) = Selector::parse(sel_str) {
            for element in document.select(&selector) {
                let mut found_vals = Vec::new();
                if let Some(v) = element.value().attr(attr) {
                    found_vals.push(v);
                }
                for (k, v) in element.value().attrs() {
                    let is_match = k.eq_ignore_ascii_case(attr)
                        || k.ends_with(&format!(":{}", attr))
                        || (sel_str == "use" && (k == "href" || k.ends_with(":href")));
                    if is_match && !found_vals.contains(&v) {
                        found_vals.push(v);
                    }
                }

                for val in found_vals {
                    let trimmed = val.trim();
                    if trimmed.starts_with('#') || trimmed.starts_with("javascript:") || trimmed.starts_with("mailto:") || trimmed.starts_with("tel:") {
                        continue;
                    }
                    if (attr == "data-bg" || attr == "data-bg-hidpi" || attr == "data-background-image") && trimmed.contains("url(") {
                        let bg_assets = extract_css_assets(trimmed, page_url);
                        assets.extend(bg_assets);
                    } else if let Ok(url) = page_url.join(trimmed) {
                        assets.push(url);
                    }
                }
            }
        }
    }

    let srcset_selectors = [
        ("[srcset]", "srcset"),
        ("[data-srcset]", "data-srcset"),
        ("[data-lazy-srcset]", "data-lazy-srcset"),
        ("[data-original-set]", "data-original-set"),
    ];
    for (sel_str, attr) in srcset_selectors {
        if let Ok(selector) = Selector::parse(sel_str) {
            for element in document.select(&selector) {
                if let Some(val) = element.value().attr(attr) {
                    for part in val.split(',') {
                        let trimmed = part.trim();
                        let url_str = trimmed.split_whitespace().next().unwrap_or("");
                        if !url_str.is_empty() && !url_str.starts_with("data:")
                            && let Ok(url) = page_url.join(url_str) {
                                assets.push(url);
                            }
                    }
                }
            }
        }
    }

    // Extract assets from <style> tags (BUG-02 / BUG-04)
    if let Ok(style_selector) = Selector::parse("style") {
        for element in document.select(&style_selector) {
            let css_text = element.text().collect::<Vec<_>>().join("");
            let style_assets = extract_css_assets(&css_text, page_url);
            assets.extend(style_assets);
        }
    }

    // Extract assets from inline style attributes (BUG-02 / BUG-04)
    if let Ok(style_attr_selector) = Selector::parse("[style]") {
        for element in document.select(&style_attr_selector) {
            if let Some(style_val) = element.value().attr("style") {
                let style_assets = extract_css_assets(style_val, page_url);
                assets.extend(style_assets);
            }
        }
    }

    assets
}

fn extract_css_assets(css_content: &str, css_url: &Url) -> Vec<Url> {
    lazy_static::lazy_static! {
        static ref URL_RE: Regex = Regex::new(
            r#"(?i)url\(\s*(?:&quot;([^&]+)&quot;|&#39;([^&]+)&#39;|&apos;([^&]+)&apos;|'([^']*)'|"([^"]*)"|([^'"\)\s]+))\s*\)"#
        ).unwrap();
        static ref IMPORT_RE: Regex = Regex::new(
            r#"(?i)@import\s+(?:url\(\s*(?:&quot;([^&]+)&quot;|'([^']*)'|"([^"]*)"|([^'"\)\s]+))\s*\)|'([^']+)'|"([^"]+)")"#
        ).unwrap();
    }
    
    let mut assets = Vec::new();
    for cap in URL_RE.captures_iter(css_content) {
        let link = cap.get(1).or_else(|| cap.get(2)).or_else(|| cap.get(3))
            .or_else(|| cap.get(4)).or_else(|| cap.get(5)).or_else(|| cap.get(6))
            .map(|m| m.as_str().trim()).unwrap_or("");
        if !link.is_empty() && !link.starts_with("data:")
            && let Ok(url) = css_url.join(link) {
                assets.push(url);
            }
    }
    for cap in IMPORT_RE.captures_iter(css_content) {
        let link = cap.get(1).or_else(|| cap.get(2)).or_else(|| cap.get(3))
            .or_else(|| cap.get(4)).or_else(|| cap.get(5)).or_else(|| cap.get(6))
            .map(|m| m.as_str().trim()).unwrap_or("");
        if !link.is_empty() && !link.starts_with("data:")
            && let Ok(url) = css_url.join(link) {
                assets.push(url);
            }
    }
    assets
}

fn rewrite_html_links(
    html: &str,
    page_url: &Url,
    base_url: &Url,
    output_dir: &Path,
    replace_special: bool,
    asset_paths: &HashMap<Url, PathBuf>,
    form_webhook: Option<&str>
) -> String {
    lazy_static::lazy_static! {
        static ref RE: Regex = Regex::new(
            r#"(?i)\b(href|src|action|data|poster|srcset|data-src|data-srcset|data-lazy-src|data-lazy-srcset|data-original|data-original-set|data-orig-file|data-large_image|data-large-file|data-medium-file|data-full-url|data-thumb|data-url|data-bg|data-bg-hidpi|data-background-image|xlink:href)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#
        ).unwrap();
        static ref STYLE_TAG_RE: Regex = Regex::new(r#"(?is)(<style\b[^>]*>)(.*?)(</style>)"#).unwrap();
        static ref STYLE_ATTR_RE: Regex = Regex::new(r#"(?i)\bstyle\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#).unwrap();
    }
    
    let current_local_path = match url_to_local_path(page_url, base_url, output_dir, replace_special, None) {
        Some(p) => p,
        None => return html.to_string(),
    };
    
    let current_dir = current_local_path.parent().unwrap_or(output_dir);

    // Step 1: Rewrite HTML attributes (href, src, srcset, data-src, etc.)
    let html_rewritten_attrs = RE.replace_all(html, |caps: &regex::Captures| {
        let attr = &caps[1];
        let link_or_srcset = caps.get(2).or_else(|| caps.get(3)).or_else(|| caps.get(4)).map(|m| m.as_str()).unwrap_or("");
        
        let attr_lower = attr.to_ascii_lowercase();

        if attr_lower == "action" && let Some(webhook) = form_webhook {
            return format!("action=\"{}\"", webhook);
        }

        if attr_lower == "srcset" || attr_lower == "data-srcset" || attr_lower == "data-lazy-srcset" || attr_lower == "data-original-set" {
            let mut rewritten_parts = Vec::new();
            for part in link_or_srcset.split(',') {
                let trimmed = part.trim();
                let mut parts_iter = trimmed.split_whitespace();
                if let Some(link) = parts_iter.next() {
                    let descriptor = parts_iter.next().unwrap_or("");
                    
                    if link.starts_with("data:") {
                        rewritten_parts.push(part.to_string());
                        continue;
                    }
                    
                    if let Ok(target_url) = page_url.join(link)
                        && is_same_or_subdomain(target_url.host_str(), base_url.host_str()) {
                            let target_local_path = asset_paths.get(&target_url).cloned()
                                .or_else(|| url_to_local_path(&target_url, base_url, output_dir, replace_special, None));
                            
                            if let Some(tlp) = target_local_path
                                && let Some(rel_path) = diff_paths(&tlp, current_dir) {
                                    let mut rel_str = rel_path.to_string_lossy().replace('\\', "/");
                                    if rel_str.is_empty() {
                                        rel_str = "./".to_string();
                                    }
                                    if descriptor.is_empty() {
                                        rewritten_parts.push(rel_str);
                                    } else {
                                        rewritten_parts.push(format!("{} {}", rel_str, descriptor));
                                    }
                                    continue;
                                }
                        }
                }
                rewritten_parts.push(part.to_string());
            }
            return format!("{}=\"{}\"", attr, rewritten_parts.join(", "));
        }

        if (attr_lower == "data-bg" || attr_lower == "data-bg-hidpi" || attr_lower == "data-background-image") && link_or_srcset.contains("url(") {
            let rewritten_css = rewrite_css_links(link_or_srcset, page_url, base_url, output_dir, replace_special, asset_paths);
            return format!("{}=\"{}\"", attr, rewritten_css);
        }
        
        let link = link_or_srcset.trim();
        
        if link.starts_with('#') || link.starts_with("data:") || link.starts_with("mailto:") || link.starts_with("tel:") || link.starts_with("javascript:") {
            return format!("{}=\"{}\"", attr, link);
        }
        
        if let Ok(target_url) = page_url.join(link)
            && is_same_or_subdomain(target_url.host_str(), base_url.host_str()) {
                let target_local_path = asset_paths.get(&target_url).cloned()
                    .or_else(|| url_to_local_path(&target_url, base_url, output_dir, replace_special, None));
                
                if let Some(target_local_path) = target_local_path
                    && let Some(rel_path) = diff_paths(&target_local_path, current_dir) {
                        let mut rel_str = rel_path.to_string_lossy().replace("\\", "/");
                        
                        if rel_str.is_empty()
                            && let Some(filename) = target_local_path.file_name() {
                                rel_str = filename.to_string_lossy().to_string();
                            }
                        
                        if let Some(fragment) = target_url.fragment() {
                            rel_str = format!("{}#{}", rel_str, fragment);
                        }
                        return format!("{}=\"{}\"", attr, rel_str);
                    }
            }
        
        format!("{}=\"{}\"", attr, link)
    }).into_owned();

    // Step 2: Rewrite <style> tags (BUG-02 / BUG-04)
    let html_rewritten_style_tags = STYLE_TAG_RE.replace_all(&html_rewritten_attrs, |caps: &regex::Captures| {
        let open_tag = &caps[1];
        let style_content = &caps[2];
        let close_tag = &caps[3];
        let rewritten_css = rewrite_css_links(style_content, page_url, base_url, output_dir, replace_special, asset_paths);
        format!("{}{}{}", open_tag, rewritten_css, close_tag)
    }).into_owned();

    // Step 3: Rewrite style="..." attributes (BUG-02 / BUG-04)
    STYLE_ATTR_RE.replace_all(&html_rewritten_style_tags, |caps: &regex::Captures| {
        if let Some(content) = caps.get(1) {
            let rewritten_css = rewrite_css_links(content.as_str(), page_url, base_url, output_dir, replace_special, asset_paths);
            format!("style=\"{}\"", rewritten_css)
        } else if let Some(content) = caps.get(2) {
            let rewritten_css = rewrite_css_links(content.as_str(), page_url, base_url, output_dir, replace_special, asset_paths);
            format!("style='{}'", rewritten_css)
        } else if let Some(content) = caps.get(3) {
            let rewritten_css = rewrite_css_links(content.as_str(), page_url, base_url, output_dir, replace_special, asset_paths);
            format!("style=\"{}\"", rewritten_css)
        } else {
            caps[0].to_string()
        }
    }).into_owned()
}

fn rewrite_css_links(
    css: &str,
    css_url: &Url,
    base_url: &Url,
    output_dir: &Path,
    replace_special: bool,
    asset_paths: &HashMap<Url, PathBuf>
) -> String {
    lazy_static::lazy_static! {
        static ref URL_RE: Regex = Regex::new(
            r#"(?i)url\(\s*(?:&quot;([^&]+)&quot;|&#39;([^&]+)&#39;|&apos;([^&]+)&apos;|'([^']*)'|"([^"]*)"|([^'"\)\s]+))\s*\)"#
        ).unwrap();
        static ref IMPORT_RE: Regex = Regex::new(
            r#"(?i)@import\s+(?:url\(\s*(?:&quot;([^&]+)&quot;|'([^']*)'|"([^"]*)"|([^'"\)\s]+))\s*\)|'([^']+)'|"([^"]+)")"#
        ).unwrap();
    }
    
    let current_local_path = match url_to_local_path(css_url, base_url, output_dir, replace_special, None) {
        Some(p) => p,
        None => return css.to_string(),
    };
    
    let current_dir = current_local_path.parent().unwrap_or(output_dir);

    let rewritten = URL_RE.replace_all(css, |caps: &regex::Captures| {
        let link = cap_str(&caps);
        
        if link.starts_with("data:") {
            return caps[0].to_string();
        }
        
        if let Ok(target_url) = css_url.join(link)
            && is_same_or_subdomain(target_url.host_str(), base_url.host_str()) {
                let target_local_path = asset_paths.get(&target_url).cloned()
                    .or_else(|| url_to_local_path(&target_url, base_url, output_dir, replace_special, None));
                
                if let Some(target_local_path) = target_local_path
                    && let Some(rel_path) = diff_paths(&target_local_path, current_dir) {
                        let mut rel_str = rel_path.to_string_lossy().replace("\\", "/");
                        
                        if rel_str.is_empty()
                            && let Some(filename) = target_local_path.file_name() {
                                rel_str = filename.to_string_lossy().to_string();
                            }
                        
                        if let Some(fragment) = target_url.fragment() {
                            rel_str = format!("{}#{}", rel_str, fragment);
                        }
                        return format!("url('{}')", rel_str);
                    }
            }
        
        caps[0].to_string()
    }).into_owned();

    IMPORT_RE.replace_all(&rewritten, |caps: &regex::Captures| {
        let link = cap_str(&caps);
        if link.starts_with("data:") {
            return caps[0].to_string();
        }
        if let Ok(target_url) = css_url.join(link)
            && is_same_or_subdomain(target_url.host_str(), base_url.host_str()) {
                let target_local_path = asset_paths.get(&target_url).cloned()
                    .or_else(|| url_to_local_path(&target_url, base_url, output_dir, replace_special, None));
                if let Some(target_local_path) = target_local_path
                    && let Some(rel_path) = diff_paths(&target_local_path, current_dir) {
                        let mut rel_str = rel_path.to_string_lossy().replace("\\", "/");
                        if rel_str.is_empty()
                            && let Some(filename) = target_local_path.file_name() {
                                rel_str = filename.to_string_lossy().to_string();
                            }
                        return format!("@import '{}'", rel_str);
                    }
            }
        caps[0].to_string()
    }).into_owned()
}

fn cap_str<'a>(caps: &'a regex::Captures<'a>) -> &'a str {
    for i in 1..caps.len() {
        if let Some(m) = caps.get(i) {
            let s = m.as_str().trim();
            if !s.is_empty() {
                return s;
            }
        }
    }
    ""
}

async fn fetch_asset_info(
    url: &Url,
    client: &Client,
    retries: usize
) -> Option<(u64, Option<String>, bool)> {
    for _ in 0..=retries {
        if let Ok(head_res) = client.head(url.clone()).send().await {
            let len = head_res.content_length().unwrap_or(0);
            let ext = head_res.headers().get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|ct| {
                    mime_guess::get_mime_extensions_str(ct).map(|exts| exts[0].to_string())
                });
            let supports_ranges = head_res.headers().get(reqwest::header::ACCEPT_RANGES)
                .is_some_and(|v| v.to_str().unwrap_or("") == "bytes");
            return Some((len, ext, supports_ranges));
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    None
}

use tokio::fs::{File, OpenOptions};
use tokio::io::{AsyncWriteExt, AsyncReadExt};
use futures_util::StreamExt;

async fn download_asset_chunked(
    client: &Client,
    asset_url: &Url,
    local_path: &Path,
    content_length: u64,
    supports_ranges: bool,
    max_size: u64,
    min_size: u64,
    retries: usize,
    is_css: bool,
) -> Result<Option<Vec<u8>>, String> {
    if content_length > max_size {
        return Err(format!("File too large: {} > {}", content_length, max_size));
    }

    let num_chunks = if supports_ranges && content_length > 1_000_000 {
        4
    } else {
        1
    };

    let mut handles = Vec::new();
    let chunk_size = if num_chunks > 1 { content_length / num_chunks } else { content_length };
    
    for i in 0..num_chunks {
        let start = i * chunk_size;
        let end = if i == num_chunks - 1 {
            content_length.saturating_sub(1)
        } else {
            start + chunk_size - 1
        };

        let part_path = local_path.with_extension(format!("{}.part{}", local_path.extension().unwrap_or_default().to_string_lossy(), i));
        
        let client_c = client.clone();
        let url_c = asset_url.clone();
        
        handles.push(tokio::spawn(async move {
            download_chunk(client_c, url_c, part_path, start, end, retries, num_chunks > 1).await
        }));
    }

    let mut success = true;
    for handle in handles {
        if let Ok(res) = handle.await {
            if res.is_err() {
                success = false;
            }
        } else {
            success = false;
        }
    }

    if !success {
        return Err("Failed to download all chunks".to_string());
    }

    let mut final_file = File::create(local_path).await.map_err(|e| e.to_string())?;
    let mut css_bytes = if is_css { Some(Vec::new()) } else { None };

    for i in 0..num_chunks {
        let part_path = local_path.with_extension(format!("{}.part{}", local_path.extension().unwrap_or_default().to_string_lossy(), i));
        let mut part_file = match File::open(&part_path).await {
            Ok(f) => f,
            Err(e) => {
                if num_chunks == 1 && content_length == 0 {
                    continue; // Might be empty file if stream didn't write anything
                }
                return Err(e.to_string());
            }
        };
        
        let mut buffer = [0; 8192];
        loop {
            let n = part_file.read(&mut buffer).await.map_err(|e| e.to_string())?;
            if n == 0 { break; }
            final_file.write_all(&buffer[..n]).await.map_err(|e| e.to_string())?;
            if let Some(ref mut cb) = css_bytes {
                cb.extend_from_slice(&buffer[..n]);
            }
        }
        
        let _ = tokio::fs::remove_file(&part_path).await;
    }
    
    if final_file.metadata().await.map(|m| m.len()).unwrap_or(0) < min_size {
         let _ = tokio::fs::remove_file(local_path).await;
         return Err("File too small".to_string());
    }

    Ok(css_bytes)
}

async fn download_chunk(
    client: Client,
    url: Url,
    part_path: std::path::PathBuf,
    start: u64,
    end: u64,
    retries: usize,
    use_range: bool,
) -> Result<(), String> {
    let mut existing_len = 0;
    if let Ok(metadata) = tokio::fs::metadata(&part_path).await {
        existing_len = metadata.len();
        if use_range && start + existing_len > end {
            return Ok(()); // Already done
        }
    }

    let actual_start = start + existing_len;

    for _ in 0..=retries {
        let mut req = client.get(url.clone()).timeout(std::time::Duration::from_secs(30));
        if use_range {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-{}", actual_start, end));
        }

        if let Ok(res) = req.send().await
            && (res.status().is_success() || res.status() == reqwest::StatusCode::PARTIAL_CONTENT) {
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&part_path)
                    .await
                    .map_err(|e| e.to_string())?;
                
                let mut stream = res.bytes_stream();
                let mut stream_success = true;
                
                while let Some(chunk_result) = stream.next().await {
                    match chunk_result {
                        Ok(chunk) => {
                            if file.write_all(&chunk).await.is_err() {
                                stream_success = false;
                                break;
                            }
                        }
                        Err(_) => {
                            stream_success = false;
                            break;
                        }
                    }
                }
                
                if stream_success {
                    return Ok(());
                }
            }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
    Err("Failed chunk".to_string())
}

#[allow(clippy::type_complexity)]
fn download_asset_recursive(
    asset_url: Url, 
    local_path: PathBuf,
    len: u64,
    supports_ranges: bool,
    base_url: Url, 
    output_dir: PathBuf, 
    client: Client, 
    args: Arc<Args>,
    file_counter: Arc<AtomicUsize>,
    semaphore: Arc<Semaphore>,
    downloaded_asset_paths: Arc<DashMap<Url, PathBuf>>
) -> Pin<Box<dyn Future<Output = Result<()>> + Send + 'static>> {
    Box::pin(async move {
        let _permit = semaphore.acquire().await.unwrap();
        if local_path.exists() { return Ok(()); }


        if args.max_files > 0 && file_counter.load(Ordering::SeqCst) >= args.max_files {
            return Ok(());
        }

        if args.dry_run {
            info!("Dry run: skipping download of asset {} to {}", asset_url.path(), local_path.display());
            return Ok(());
        }

        if local_path.exists() {
            return Ok(()); 
        }

        if len > args.max_size {
            debug!("Skipping {} (exceeds max size)", asset_url);
            return Ok(());
        }
        if len < args.min_size && len > 0 {
            debug!("Skipping {} (below min size)", asset_url);
            return Ok(());
        }

        if let Some(parent) = local_path.parent() {
            fs::create_dir_all(parent)?;
        }

        let is_css = local_path.extension().is_some_and(|e| e == "css");

        let download_result = download_asset_chunked(
            &client,
            &asset_url,
            &local_path,
            len,
            supports_ranges,
            args.max_size,
            args.min_size,
            args.retries,
            is_css,
        ).await;

        let css_bytes = match download_result {
            Ok(b) => {
                        b
            },
            Err(e) => {
                warn!("Failed to download asset completely: {} - {}", asset_url, e);
                return Ok(());
            }
        };

        if is_css
            && let Some(bytes) = css_bytes
                && let Ok(css_str) = std::str::from_utf8(&bytes) {
                    let inner_assets = extract_css_assets(css_str, &asset_url);
                    let mut inner_asset_paths = HashMap::new();
                    
                    let mut fetch_futures = Vec::new();
                    let mut unique_inner = std::collections::HashSet::new();
                    for a in inner_assets { unique_inner.insert(a); }
                    for inner_url in unique_inner {
                        if !is_same_or_subdomain(inner_url.host_str(), base_url.host_str()) { continue; }
                        let client_c = client.clone();
                        let args_c = Arc::clone(&args);
                        let dl_assets_c = Arc::clone(&downloaded_asset_paths);
                        let base_url_c = base_url.clone();
                        let output_dir_c = output_dir.clone();
                        
                        fetch_futures.push(tokio::spawn(async move {
                            let mut needs_download = false;
                            let mut dl_info = None;
                            
                            let local_path = if let Some(entry) = dl_assets_c.get(&inner_url) {
                                entry.value().clone()
                            } else {
                                let (len, mut ext, supports_ranges) = fetch_asset_info(&inner_url, &client_c, args_c.retries).await.unwrap_or((0, None, false));
                                if inner_url.path().split("/").last().map(|s| s.contains(".")).unwrap_or(false) {
                                    ext = None;
                                }
                                if let Some(lp) = url_to_local_path(&inner_url, &base_url_c, &output_dir_c, args_c.replace_special_chars, ext.as_deref()) {
                                    dl_assets_c.insert(inner_url.clone(), lp.clone());
                                    needs_download = true;
                                    dl_info = Some((len, supports_ranges));
                                    lp
                                } else {
                                    return None;
                                }
                            };
                            Some((inner_url, local_path, needs_download, dl_info))
                        }));
                    }
                    
                    let mut to_download = Vec::new();
                    for fut in fetch_futures {
                        if let Ok(Some((u, lp, needs_dl, info))) = fut.await {
                            inner_asset_paths.insert(u.clone(), lp.clone());
                            if needs_dl {
                                to_download.push((u, lp, info.unwrap()));
                            }
                        }
                    }
                    
                    let rewritten_css = rewrite_css_links(css_str, &asset_url, &base_url, &output_dir, args.replace_special_chars, &inner_asset_paths);
                    let _ = fs::write(&local_path, rewritten_css);
                    
                    for (u, lp, (l, sr)) in to_download {
                        let client_clone = client.clone();
                        let base_url_clone = base_url.clone();
                        let output_dir_clone = output_dir.clone();
                        let args_clone = Arc::clone(&args);
                        let fc = Arc::clone(&file_counter);
                        let sem_clone = Arc::clone(&semaphore);
                        let dl_assets_clone = Arc::clone(&downloaded_asset_paths);
                        
                        tokio::spawn(async move {
                            let _ = download_asset_recursive(
                                u, lp, l, sr, base_url_clone, output_dir_clone, 
                                client_clone, args_clone, fc, sem_clone, dl_assets_clone
                            ).await;
                        });
                    }
                }
        
        file_counter.fetch_add(1, Ordering::SeqCst);

        Ok(())
    })
}

fn handle_base_tag(html_content: &str, page_url: &Url) -> (String, Url) {
    let document = Html::parse_document(html_content);
    let base_selector = Selector::parse("base[href]").unwrap();
    
    if let Some(base_el) = document.select(&base_selector).next()
        && let Some(href) = base_el.value().attr("href")
            && let Ok(new_base) = page_url.join(href) {
                let re = Regex::new(r#"(?i)<base\s+[^>]*href\s*=\s*['"]?([^'">]+)['"]?[^>]*>"#).unwrap();
                let new_html = re.replace_all(html_content, "").to_string();
                return (new_html, new_base);
            }
    (html_content.to_string(), page_url.clone())
}

fn scroll_page_to_bottom(tab: &headless_chrome::Tab) {
    let scroll_script = r#"
        async () => {
            await new Promise((resolve) => {
                let totalHeight = 0;
                let distance = 300;
                let timer = setInterval(() => {
                    let scrollHeight = document.body.scrollHeight;
                    window.scrollBy(0, distance);
                    totalHeight += distance;
                    if (totalHeight >= scrollHeight) {
                        clearInterval(timer);
                        window.scrollTo(0, 0);
                        resolve();
                    }
                }, 100);
            });
        }
    "#;
    let _ = tab.evaluate(scroll_script, true);
    std::thread::sleep(std::time::Duration::from_millis(500));
}

fn percent_decode_path(path: &str) -> String {
    let mut bytes = Vec::new();
    let mut chars = path.bytes();
    while let Some(b) = chars.next() {
        if b == b'%' {
            let h1 = chars.next();
            let h2 = chars.next();
            if let (Some(h1), Some(h2)) = (h1, h2) {
                if let Ok(hex_str) = std::str::from_utf8(&[h1, h2]) {
                    if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                        bytes.push(val);
                        continue;
                    }
                }
            }
        }
        bytes.push(b);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn resolve_preview_file(base_dir: &Path, rel_path: &str, spa_mode: bool) -> (Option<PathBuf>, u16) {
    if rel_path.is_empty() {
        let index = base_dir.join("index.html");
        if index.is_file() {
            return (Some(index), 200);
        }
    }

    let direct = base_dir.join(rel_path);
    if direct.is_file() {
        return (Some(direct), 200);
    }
    if direct.is_dir() {
        let index = direct.join("index.html");
        if index.is_file() {
            return (Some(index), 200);
        }
    }

    let html_candidate = base_dir.join(format!("{}.html", rel_path));
    if html_candidate.is_file() {
        return (Some(html_candidate), 200);
    }

    let nested_index = base_dir.join(rel_path).join("index.html");
    if nested_index.is_file() {
        return (Some(nested_index), 200);
    }

    if spa_mode {
        let root_index = base_dir.join("index.html");
        if root_index.is_file() {
            return (Some(root_index), 200);
        }
    }

    let not_found_page = base_dir.join("404.html");
    if not_found_page.is_file() {
        return (Some(not_found_page), 404);
    }

    (None, 404)
}

async fn run_preview_server(dir: &Path, host: &str, port: u16, spa_mode: bool) -> Result<()> {
    let addr = format!("{}:{}", host, port);
    let listener = tokio::net::TcpListener::bind(&addr).await
        .context(format!("Failed to bind preview server to {}", addr))?;

    let canonical_dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());

    println!("{}", "========================================".cyan().bold());
    println!("{} http://{}", "🚀 Local Preview Server:".green().bold(), addr.yellow());
    println!("{} {}", "📁 Serving directory:".green().bold(), canonical_dir.display().to_string().yellow());
    if spa_mode {
        println!("{} Enabled", "⚡ SPA Fallback:".green().bold());
    }
    println!("{}", "========================================\n".cyan().bold());

    let serve_dir = Arc::new(canonical_dir);

    loop {
        let (mut socket, _) = match listener.accept().await {
            Ok(conn) => conn,
            Err(e) => {
                tracing::warn!("Accept error: {}", e);
                continue;
            }
        };
        let serve_dir = Arc::clone(&serve_dir);

        tokio::spawn(async move {
            let mut buf = [0u8; 4096];
            let n = match socket.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };

            let req_str = String::from_utf8_lossy(&buf[..n]);
            let first_line = req_str.lines().next().unwrap_or("");
            let parts: Vec<&str> = first_line.split_whitespace().collect();
            if parts.len() < 2 || (parts[0] != "GET" && parts[0] != "HEAD") {
                let _ = socket.write_all(b"HTTP/1.1 405 Method Not Allowed\r\n\r\n").await;
                return;
            }

            let raw_path = parts[1].split('?').next().unwrap_or("/");
            let decoded_path = percent_decode_path(raw_path);
            let rel_path = decoded_path.trim_start_matches('/');

            let (target_path, status_code) = resolve_preview_file(&serve_dir, rel_path, spa_mode);

            if let Some(file_path) = target_path {
                let content_type = mime_guess::from_path(&file_path)
                    .first_or_octet_stream()
                    .to_string();
                if let Ok(body) = tokio::fs::read(&file_path).await {
                    let status_line = match status_code {
                        200 => "HTTP/1.1 200 OK",
                        404 => "HTTP/1.1 404 Not Found",
                        _ => "HTTP/1.1 200 OK",
                    };
                    let header = format!(
                        "{}\r\nContent-Type: {}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                        status_line, content_type, body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    if parts[0] == "GET" {
                        let _ = socket.write_all(&body).await;
                    }
                    return;
                }
            }

            let not_found_body = b"<!DOCTYPE html><html><body><h1>404 Not Found</h1></body></html>";
            let header = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n",
                not_found_body.len()
            );
            let _ = socket.write_all(header.as_bytes()).await;
            if parts[0] == "GET" {
                let _ = socket.write_all(not_found_body).await;
            }
        });
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Default, Clone)]
struct FidelityAuditReport {
    target_host: String,
    backup_dir: String,
    pages_checked: usize,
    total_links_checked: usize,
    origin_leaks: usize,
    root_relative_breaks: usize,
    broken_local_links: usize,
    missing_assets: usize,
    fidelity_score: f64,
    grade: String,
    leak_samples: Vec<String>,
    root_relative_samples: Vec<String>,
    broken_link_samples: Vec<String>,
    missing_asset_samples: Vec<String>,
}

fn run_fidelity_audit(backup_dir: &Path, base_host: Option<&str>) -> Result<FidelityAuditReport> {
    let mut report = FidelityAuditReport {
        target_host: base_host.unwrap_or("unknown").to_string(),
        backup_dir: backup_dir.display().to_string(),
        ..Default::default()
    };

    if !backup_dir.exists() {
        anyhow::bail!("Directory does not exist: {}", backup_dir.display());
    }

    let mut html_files = Vec::new();
    let mut css_files = Vec::new();

    fn collect_files(dir: &Path, htmls: &mut Vec<PathBuf>, csses: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    collect_files(&path, htmls, csses);
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if ext.eq_ignore_ascii_case("html") || ext.eq_ignore_ascii_case("htm") {
                        htmls.push(path);
                    } else if ext.eq_ignore_ascii_case("css") {
                        csses.push(path);
                    }
                }
            }
        }
    }

    collect_files(backup_dir, &mut html_files, &mut css_files);
    report.pages_checked = html_files.len();

    lazy_static::lazy_static! {
        static ref AUDIT_ATTR_RE: Regex = Regex::new(
            r#"(?i)\b(href|src|action|data|poster|srcset|data-src|data-srcset|data-lazy-src|data-lazy-srcset|data-original|data-original-set|data-orig-file|data-large_image|data-large-file|data-medium-file|data-full-url|data-thumb|data-url|data-bg|data-bg-hidpi|data-background-image|xlink:href)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#
        ).unwrap();
    }

    let base_clean = base_host.map(clean_host);

    for html_file in &html_files {
        let content = match fs::read_to_string(html_file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let current_dir = html_file.parent().unwrap_or(backup_dir);

        for cap in AUDIT_ATTR_RE.captures_iter(&content) {
            let attr = &cap[1];
            let raw_val = cap.get(2).or_else(|| cap.get(3)).or_else(|| cap.get(4)).map(|m| m.as_str().trim()).unwrap_or("");
            if raw_val.is_empty() || raw_val.starts_with('#') || raw_val.starts_with("data:") || raw_val.starts_with("javascript:") || raw_val.starts_with("mailto:") || raw_val.starts_with("tel:") {
                continue;
            }

            let values_to_check: Vec<&str> = if attr.to_ascii_lowercase().contains("srcset") || attr.to_ascii_lowercase().contains("original-set") {
                raw_val.split(',').filter_map(|p| p.trim().split_whitespace().next()).collect()
            } else {
                vec![raw_val]
            };

            for val in values_to_check {
                report.total_links_checked += 1;

                // 1. Origin Leak Check
                if let Some(bh) = base_clean {
                    if val.starts_with("http://") || val.starts_with("https://") || val.starts_with("//") {
                        let parsed_host = if let Ok(u) = Url::parse(val) {
                            u.host_str().map(|h| h.to_string())
                        } else if val.starts_with("//") {
                            Url::parse(&format!("https:{}", val)).ok().and_then(|u| u.host_str().map(|h| h.to_string()))
                        } else {
                            None
                        };

                        if let Some(ref ph) = parsed_host {
                            if is_same_or_subdomain(Some(ph), Some(bh)) {
                                report.origin_leaks += 1;
                                if report.leak_samples.len() < 10 {
                                    report.leak_samples.push(format!("{}: {}='{}'", html_file.display(), attr, val));
                                }
                                continue;
                            }
                        }
                    }
                }

                // 2. Root-Relative Break Check
                if val.starts_with('/') && !val.starts_with("//") {
                    report.root_relative_breaks += 1;
                    if report.root_relative_samples.len() < 10 {
                        report.root_relative_samples.push(format!("{}: {}='{}'", html_file.display(), attr, val));
                    }
                    continue;
                }

                // 3. Broken Local Link / Missing Asset Check
                if !val.starts_with("http://") && !val.starts_with("https://") && !val.starts_with("//") {
                    let clean_val = val.split('?').next().unwrap_or(val).split('#').next().unwrap_or(val);
                    let target_path = if clean_val.is_empty() || clean_val == "." {
                        current_dir.join("index.html")
                    } else {
                        let resolved = current_dir.join(clean_val);
                        if resolved.is_dir() {
                            resolved.join("index.html")
                        } else {
                            resolved
                        }
                    };

                    let exists = target_path.exists()
                        || (clean_val.ends_with('/') && current_dir.join(clean_val).join("index.html").exists())
                        || current_dir.join(format!("{}.html", clean_val)).exists();

                    if !exists {
                        let is_asset = attr.eq_ignore_ascii_case("src") || attr.eq_ignore_ascii_case("poster")
                            || clean_val.ends_with(".png") || clean_val.ends_with(".jpg") || clean_val.ends_with(".jpeg")
                            || clean_val.ends_with(".css") || clean_val.ends_with(".js") || clean_val.ends_with(".svg")
                            || clean_val.ends_with(".woff2") || clean_val.ends_with(".woff");

                        if is_asset {
                            report.missing_assets += 1;
                            if report.missing_asset_samples.len() < 10 {
                                report.missing_asset_samples.push(format!("{}: {}='{}'", html_file.display(), attr, val));
                            }
                        } else {
                            report.broken_local_links += 1;
                            if report.broken_link_samples.len() < 10 {
                                report.broken_link_samples.push(format!("{}: {}='{}'", html_file.display(), attr, val));
                            }
                        }
                    }
                }
            }
        }
    }

    // Calculate score
    let score = if report.total_links_checked == 0 {
        100.0
    } else {
        let penalty = (report.origin_leaks as f64 * 1.5 + report.root_relative_breaks as f64 * 1.0 + report.broken_local_links as f64 * 1.2 + report.missing_assets as f64 * 0.8) / (report.total_links_checked as f64) * 100.0;
        (100.0 - penalty).clamp(0.0, 100.0)
    };

    report.fidelity_score = (score * 10.0).round() / 10.0;
    report.grade = match report.fidelity_score {
        s if s >= 95.0 => "A+".to_string(),
        s if s >= 90.0 => "A".to_string(),
        s if s >= 80.0 => "B".to_string(),
        s if s >= 70.0 => "C".to_string(),
        s if s >= 55.0 => "D".to_string(),
        _ => "F".to_string(),
    };

    // Print Report
    println!("\n{}", "========================================".cyan().bold());
    println!("{} {}", "📊 CLONE FIDELITY AUDIT REPORT:".green().bold(), report.target_host.yellow());
    println!("{}", "========================================".cyan().bold());
    let score_colored = if report.fidelity_score >= 90.0 {
        format!("{:.1}% [GRADE: {}]", report.fidelity_score, report.grade).green().bold()
    } else if report.fidelity_score >= 70.0 {
        format!("{:.1}% [GRADE: {}]", report.fidelity_score, report.grade).yellow().bold()
    } else {
        format!("{:.1}% [GRADE: {}]", report.fidelity_score, report.grade).red().bold()
    };
    println!("{} {}", "🎯 Overall Fidelity Score:".bold(), score_colored);
    println!("{} {}", "📄 Pages Evaluated:".bold(), report.pages_checked.to_string().cyan());
    println!("{} {}", "🔗 Links Checked:".bold(), report.total_links_checked.to_string().cyan());
    println!("");
    println!("  {} Live Origin Leaks:        {}", if report.origin_leaks == 0 { "✔".green() } else { "✖".red() }, report.origin_leaks);
    println!("  {} Root-Relative Breaks:     {}", if report.root_relative_breaks == 0 { "✔".green() } else { "✖".red() }, report.root_relative_breaks);
    println!("  {} Broken Local Links (404): {}", if report.broken_local_links == 0 { "✔".green() } else { "✖".red() }, report.broken_local_links);
    println!("  {} Missing Assets:           {}", if report.missing_assets == 0 { "✔".green() } else { "✖".red() }, report.missing_assets);
    println!("{}", "========================================\n".cyan().bold());

    // Write markdown and json reports
    let md_path = backup_dir.join("audit_report.md");
    let json_path = backup_dir.join("audit_report.json");

    let mut md_content = format!(
        "# Clone Fidelity Audit Report: {}\n\n- **Date**: {}\n- **Fidelity Score**: **{:.1}%** [Grade: **{}**]\n- **Pages Checked**: {}\n- **Total Links Checked**: {}\n- **Origin Leaks**: {}\n- **Root-Relative Breaks**: {}\n- **Broken Local Links**: {}\n- **Missing Assets**: {}\n\n",
        report.target_host,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%SZ"),
        report.fidelity_score,
        report.grade,
        report.pages_checked,
        report.total_links_checked,
        report.origin_leaks,
        report.root_relative_breaks,
        report.broken_local_links,
        report.missing_assets
    );

    if !report.leak_samples.is_empty() {
        md_content.push_str("### ⚠️ Sample Origin Leaks\n```text\n");
        for s in &report.leak_samples { md_content.push_str(&format!("{}\n", s)); }
        md_content.push_str("```\n\n");
    }
    if !report.missing_asset_samples.is_empty() {
        md_content.push_str("### ⚠️ Sample Missing Assets\n```text\n");
        for s in &report.missing_asset_samples { md_content.push_str(&format!("{}\n", s)); }
        md_content.push_str("```\n\n");
    }

    let _ = fs::write(&md_path, md_content);
    if let Ok(json_str) = serde_json::to_string_pretty(&report) {
        let _ = fs::write(&json_path, json_str);
    }

    Ok(report)
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
struct SearchIndexItem {
    url: String,
    title: String,
    description: String,
    headings: Vec<String>,
}

fn build_search_index(html: &str, page_url_str: &str) -> Option<SearchIndexItem> {
    let document = Html::parse_document(html);
    let title_sel = Selector::parse("title").ok()?;
    let title = document.select(&title_sel).next().map(|e| e.inner_html()).unwrap_or_default();
    let meta_desc_sel = Selector::parse("meta[name='description']").ok()?;
    let description = document.select(&meta_desc_sel).next().and_then(|e| e.value().attr("content")).unwrap_or("").to_string();

    let mut headings = Vec::new();
    if let Ok(h_sel) = Selector::parse("h1, h2, h3") {
        for el in document.select(&h_sel) {
            let t = el.text().collect::<Vec<_>>().join(" ").trim().to_string();
            if !t.is_empty() && t.len() < 120 {
                headings.push(t);
            }
        }
    }

    Some(SearchIndexItem {
        url: page_url_str.to_string(),
        title,
        description,
        headings,
    })
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
struct FacebookPost {
    post_id: String,
    author: String,
    timestamp: String,
    text: String,
    likes: usize,
    comments_count: usize,
    shares_count: usize,
    media_urls: Vec<String>,
    local_media_paths: Vec<String>,
    post_url: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default)]
struct FacebookPageArchive {
    page_name: String,
    page_url: String,
    archived_at: String,
    total_posts: usize,
    posts: Vec<FacebookPost>,
}

fn parse_facebook_html(html: &str, page_url: &str) -> Vec<FacebookPost> {
    let document = Html::parse_document(html);
    let mut posts = Vec::new();

    let post_selectors = [
        "div[role='feed'] > div",
        "div[data-pagelet^='FeedUnit']",
        "div[data-ad-preview='message']",
        "article",
        "div.story_body_container",
        "div._4-u2._4-u8",
    ];

    let message_selector = Selector::parse("div[data-ad-preview='message'], div[data-ad-comet-preview='message'], div[dir='auto']").unwrap();
    let img_selector = Selector::parse("img[src*='fbcdn'], img[src*='scontent'], img.scaledImageFitWidth, img.scaledImageFitHeight, img[data-src]").unwrap();
    let link_selector = Selector::parse("a[href*='/posts/'], a[href*='/photos/'], a[href*='/videos/'], a[href*='story_fbid']").unwrap();

    let mut found_containers = false;
    for sel_str in &post_selectors {
        if let Ok(sel) = Selector::parse(sel_str) {
            for (idx, el) in document.select(&sel).enumerate() {
                let post_html = el.html();
                let sub_doc = Html::parse_fragment(&post_html);
                
                let mut text_parts = Vec::new();
                for msg_el in sub_doc.select(&message_selector) {
                    let t = msg_el.text().collect::<Vec<_>>().join(" ").trim().to_string();
                    if !t.is_empty() && !text_parts.contains(&t) {
                        text_parts.push(t);
                    }
                }
                let text = text_parts.join("\n\n");

                let mut media_urls = Vec::new();
                for img_el in sub_doc.select(&img_selector) {
                    if let Some(src) = img_el.value().attr("src").or_else(|| img_el.value().attr("data-src")) {
                        if (src.contains("fbcdn.net") || src.contains("scontent")) && !src.contains("emoji.php") && !src.contains("rsrc.php") {
                            let clean_src = src.replace("&amp;", "&");
                            if !media_urls.contains(&clean_src) {
                                media_urls.push(clean_src);
                            }
                        }
                    }
                }

                if text.is_empty() && media_urls.is_empty() {
                    continue;
                }

                found_containers = true;
                let post_url = sub_doc.select(&link_selector).next()
                    .and_then(|a| a.value().attr("href"))
                    .map(|h| if h.starts_with("http") { h.to_string() } else { format!("https://www.facebook.com{}", h) })
                    .unwrap_or_else(|| format!("{}#post-{}", page_url, idx + 1));

                posts.push(FacebookPost {
                    post_id: format!("post_{}", idx + 1),
                    author: page_url.trim_end_matches('/').split('/').last().unwrap_or("Facebook Page").to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    text,
                    likes: 0,
                    comments_count: 0,
                    shares_count: 0,
                    media_urls,
                    local_media_paths: Vec::new(),
                    post_url,
                });
            }
            if found_containers && !posts.is_empty() {
                break;
            }
        }
    }

    if posts.is_empty() {
        for (idx, msg_el) in document.select(&message_selector).enumerate() {
            let text = msg_el.text().collect::<Vec<_>>().join(" ").trim().to_string();
            if text.len() > 10 {
                posts.push(FacebookPost {
                    post_id: format!("post_{}", idx + 1),
                    author: page_url.trim_end_matches('/').split('/').last().unwrap_or("Facebook Page").to_string(),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                    text,
                    likes: 0,
                    comments_count: 0,
                    shares_count: 0,
                    media_urls: Vec::new(),
                    local_media_paths: Vec::new(),
                    post_url: format!("{}#post-{}", page_url, idx + 1),
                });
            }
        }
    }

    posts
}

fn generate_facebook_static_feed(archive: &FacebookPageArchive) -> String {
    let mut posts_html = String::new();
    for post in &archive.posts {
        let mut media_html = String::new();
        for media_path in &post.local_media_paths {
            media_html.push_str(&format!(
                r#"<div class="post-media"><img src="{}" alt="Post media" loading="lazy"></div>"#,
                media_path
            ));
        }
        if media_html.is_empty() && !post.media_urls.is_empty() {
            for url in &post.media_urls {
                media_html.push_str(&format!(
                    r#"<div class="post-media"><img src="{}" alt="Post media" loading="lazy"></div>"#,
                    url
                ));
            }
        }

        let formatted_text = post.text.replace('\n', "<br>");

        posts_html.push_str(&format!(
            r#"
            <article class="fb-post-card" id="{}">
                <header class="post-header">
                    <div class="author-avatar">📘</div>
                    <div class="author-meta">
                        <h3 class="author-name">{}</h3>
                        <time class="post-time">{}</time>
                    </div>
                </header>
                <div class="post-content">
                    <p>{}</p>
                    {}
                </div>
                <footer class="post-footer">
                    <div class="engagement-stats">
                        <span>👍 {} Likes</span>
                        <span>💬 {} Comments</span>
                        <span>🔄 {} Shares</span>
                    </div>
                    <a href="{}" target="_blank" rel="noopener" class="original-link">View on Facebook →</a>
                </footer>
            </article>
            "#,
            post.post_id,
            post.author,
            post.timestamp,
            formatted_text,
            media_html,
            post.likes,
            post.comments_count,
            post.shares_count,
            post.post_url
        ));
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{} - Facebook Archive</title>
    <style>
        :root {{ --fb-blue: #1877f2; --bg-main: #f0f2f5; --card-bg: #ffffff; --text-primary: #050505; --text-secondary: #65676b; --border-color: #ced0d4; }}
        body {{ font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif; background: var(--bg-main); color: var(--text-primary); margin: 0; padding: 20px; display: flex; flex-direction: column; align-items: center; }}
        .header-container {{ width: 100%; max-width: 680px; background: var(--card-bg); border-radius: 8px; box-shadow: 0 1px 2px rgba(0,0,0,0.2); padding: 20px; margin-bottom: 20px; box-sizing: border-box; }}
        .header-title {{ font-size: 24px; font-weight: bold; margin: 0 0 8px 0; color: var(--fb-blue); }}
        .header-meta {{ font-size: 14px; color: var(--text-secondary); }}
        .feed-container {{ width: 100%; max-width: 680px; display: flex; flex-direction: column; gap: 16px; }}
        .fb-post-card {{ background: var(--card-bg); border-radius: 8px; box-shadow: 0 1px 2px rgba(0,0,0,0.2); padding: 16px; display: flex; flex-direction: column; gap: 12px; }}
        .post-header {{ display: flex; align-items: center; gap: 10px; }}
        .author-avatar {{ font-size: 28px; width: 40px; height: 40px; display: flex; align-items: center; justify-content: center; background: #e7f3ff; border-radius: 50%; }}
        .author-name {{ margin: 0; font-size: 16px; font-weight: 600; }}
        .post-time {{ font-size: 12px; color: var(--text-secondary); }}
        .post-content p {{ margin: 0; line-height: 1.4; white-space: pre-wrap; }}
        .post-media {{ margin-top: 10px; border-radius: 6px; overflow: hidden; max-height: 500px; }}
        .post-media img {{ width: 100%; height: auto; object-fit: cover; display: block; }}
        .post-footer {{ border-top: 1px solid var(--border-color); padding-top: 10px; display: flex; justify-content: space-between; align-items: center; font-size: 13px; color: var(--text-secondary); }}
        .engagement-stats {{ display: flex; gap: 12px; }}
        .original-link {{ color: var(--fb-blue); text-decoration: none; font-weight: 500; }}
        .original-link:hover {{ text-decoration: underline; }}
    </style>
</head>
<body>
    <header class="header-container">
        <h1 class="header-title">{}</h1>
        <div class="header-meta">Archived on {} • Total Posts: {}</div>
    </header>
    <main class="feed-container">
        {}
    </main>
</body>
</html>"#,
        archive.page_name,
        archive.page_name,
        archive.archived_at,
        archive.total_posts,
        posts_html
    )
}

async fn run_facebook_archive(
    page_url: &str,
    output_dir: &Path,
    max_posts: Option<usize>,
    _extract_data: bool,
    client: &Client,
    browser: Option<Arc<headless_chrome::Browser>>,
) -> Result<()> {
    println!("{}", "========================================".cyan().bold());
    println!("{} {}", "📘 Starting Facebook Archive:".green().bold(), page_url.yellow());
    println!("{} {}", "📁 Output Directory:".green().bold(), output_dir.display().to_string().yellow());
    println!("{}", "========================================\n".cyan().bold());

    fs::create_dir_all(output_dir)?;
    let media_dir = output_dir.join("media");
    fs::create_dir_all(&media_dir)?;

    let mut html_content = String::new();

    if let Some(b) = &browser {
        if let Ok(tab) = b.new_tab() {
            if tab.navigate_to(page_url).is_ok() && tab.wait_until_navigated().is_ok() {
                let scroll_script = r#"
                    new Promise((resolve) => {
                        let distance = 500;
                        let count = 0;
                        let timer = setInterval(() => {
                            window.scrollBy(0, distance);
                            count++;
                            if (count >= 15) {
                                clearInterval(timer);
                                resolve();
                            }
                        }, 250);
                    });
                "#;
                let _ = tab.evaluate(scroll_script, true);
                std::thread::sleep(std::time::Duration::from_millis(1500));
                if let Ok(c) = tab.get_content() {
                    html_content = c;
                }
            }
        }
    }

    if html_content.is_empty() {
        if let Ok(resp) = client.get(page_url).send().await {
            if let Ok(text) = resp.text().await {
                html_content = text;
            }
        }
    }

    if html_content.is_empty() {
        anyhow::bail!("Failed to fetch Facebook page HTML from {}", page_url);
    }

    let mut posts = parse_facebook_html(&html_content, page_url);
    if let Some(limit) = max_posts {
        if posts.len() > limit {
            posts.truncate(limit);
        }
    }

    println!("Found {} posts. Downloading media assets...", posts.len().to_string().cyan());

    for post in &mut posts {
        for (m_idx, media_url) in post.media_urls.iter().enumerate() {
            let ext = if media_url.contains(".png") { "png" } else { "jpg" };
            let filename = format!("{}_media_{}.{}", post.post_id, m_idx + 1, ext);
            let local_path = media_dir.join(&filename);
            let rel_path = format!("media/{}", filename);

            if let Ok(resp) = client.get(media_url).send().await {
                if let Ok(bytes) = resp.bytes().await {
                    let _ = fs::write(&local_path, &bytes);
                    post.local_media_paths.push(rel_path);
                }
            }
        }
    }

    let page_name = page_url.trim_end_matches('/').split('/').last().unwrap_or("Facebook Page").to_string();
    let archive = FacebookPageArchive {
        page_name: page_name.clone(),
        page_url: page_url.to_string(),
        archived_at: chrono::Utc::now().to_rfc3339(),
        total_posts: posts.len(),
        posts: posts.clone(),
    };

    let feed_html = generate_facebook_static_feed(&archive);
    fs::write(output_dir.join("index.html"), feed_html)?;

    let json_str = serde_json::to_string_pretty(&archive)?;
    fs::write(output_dir.join("posts.json"), json_str)?;

    let csv_path = output_dir.join("posts.csv");
    if let Ok(mut w) = csv::Writer::from_path(&csv_path) {
        let _ = w.write_record(&["post_id", "author", "timestamp", "text", "likes", "comments", "shares", "post_url", "media_urls"]);
        for p in &posts {
            let _ = w.write_record(&[
                &p.post_id,
                &p.author,
                &p.timestamp,
                &p.text,
                &p.likes.to_string(),
                &p.comments_count.to_string(),
                &p.shares_count.to_string(),
                &p.post_url,
                &p.local_media_paths.join(";"),
            ]);
        }
        let _ = w.flush();
    }

    println!("{}", "========================================".cyan().bold());
    println!("🎉 {}!", "Facebook Archival Complete".green().bold());
    println!("📄 Archived {} posts to {}", posts.len().to_string().cyan(), output_dir.display().to_string().yellow());
    println!("🌐 Static offline feed: {}", output_dir.join("index.html").display().to_string().yellow());
    println!("📊 Structured data saved to posts.json & posts.csv");
    println!("{}", "========================================\n".cyan().bold());

    Ok(())
}

fn main() -> Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(8 * 1024 * 1024)
        .build()?
        .block_on(async_main())
}

async fn async_main() -> Result<()> {
    let start_time = std::time::Instant::now();
    let args = Arc::new(Args::parse());

    let is_serve_mode = args.url.eq_ignore_ascii_case("serve") || args.serve.is_some();
    if is_serve_mode {
        let dir_str = if let Some(Some(dir)) = &args.serve {
            dir.clone()
        } else if let Some(dir) = &args.serve_dir {
            dir.clone()
        } else if let Some(output) = &args.output {
            output.clone()
        } else {
            ".".to_string()
        };
        let serve_path = PathBuf::from(dir_str);
        return run_preview_server(&serve_path, &args.host, args.port, args.spa).await;
    }

    let is_audit_mode = args.url.eq_ignore_ascii_case("audit");
    if is_audit_mode {
        let audit_dir_str = args.serve_dir.clone().or_else(|| args.output.clone()).unwrap_or_else(|| ".".to_string());
        let audit_path = PathBuf::from(audit_dir_str);
        let host_hint = args.live_url.as_deref().and_then(|u| Url::parse(u).ok()).and_then(|u| u.host_str().map(|s| s.to_string()));
        let _ = run_fidelity_audit(&audit_path, host_hint.as_deref())?;
        return Ok(());
    }

    let is_fb_mode = args.url.eq_ignore_ascii_case("facebook") || args.url.eq_ignore_ascii_case("social");
    if is_fb_mode {
        let target_fb_url = args.serve_dir.clone().unwrap_or_default();
        if target_fb_url.is_empty() {
            eprintln!("{}: {}", "Error".red().bold(), "Please provide a Facebook page URL (e.g. 'frontlane-static facebook https://facebook.com/Nike --output ./nike-fb')");
            std::process::exit(1);
        }
        let out_dir_name = args.output.clone().unwrap_or_else(|| "facebook-archive".to_string());
        let out_dir = PathBuf::from(out_dir_name);
        let client = Client::builder()
            .user_agent(&args.user_agent)
            .timeout(std::time::Duration::from_secs(args.timeout))
            .build()?;
        let mut browser = None;
        if let Ok(b) = headless_chrome::Browser::default() {
            browser = Some(Arc::new(b));
        }
        return run_facebook_archive(&target_fb_url, &out_dir, args.max_posts, args.extract_data, &client, browser).await;
    }

    if args.url.is_empty() {
        eprintln!("{}: {}", "Error".red().bold(), "URL argument is required (e.g. 'frontlane-static https://example.com' or 'frontlane-static serve ./backup' or 'frontlane-static audit ./backup')");
        std::process::exit(1);
    }

    let file_appender = tracing_appender::rolling::never(".", "spider.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

    if args.verbose {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(tracing::Level::DEBUG)
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    } else {
        let subscriber = FmtSubscriber::builder()
            .with_max_level(tracing::Level::INFO)
            .with_writer(non_blocking)
            .finish();
        tracing::subscriber::set_global_default(subscriber).expect("setting default subscriber failed");
    }
    let base_url = Url::parse(&args.url).context("Invalid URL")?;
    
    let output_dir_name = args.output.clone().unwrap_or_else(|| {
        base_url.host_str().unwrap_or("backup").to_string()
    });
    
    let output_dir = Path::new(&output_dir_name).to_path_buf();
    
    println!("{}", "========================================".cyan().bold());
    println!("{} {}", "🚀 Starting Site Backup:".green().bold(), args.url.yellow());
    println!("{} {}", "📁 Output Directory:".green().bold(), output_dir.display().to_string().yellow());
    println!("{}", "========================================\n".cyan().bold());
    
    let multi_progress = Arc::new(MultiProgress::new());
    let main_pb = multi_progress.add(ProgressBar::new(100)); // Will update length after sitemap
    main_pb.set_style(ProgressStyle::default_bar()
        .template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} pages ({eta}) {msg}")
        .unwrap()
        .progress_chars("#>-"));
    main_pb.set_message("Crawling website...");
    main_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    if !args.dry_run {
        fs::create_dir_all(&output_dir)?;
    }

    let mut headers = reqwest::header::HeaderMap::new();
    if let Some(auth) = &args.auth {
        use base64::{engine::general_purpose, Engine as _};
        let b64 = general_purpose::STANDARD.encode(auth);
        if let Ok(val) = reqwest::header::HeaderValue::from_str(&format!("Basic {}", b64)) {
            headers.insert(reqwest::header::AUTHORIZATION, val);
        }
    }
    if let Some(cookie) = &args.cookie
        && let Ok(val) = reqwest::header::HeaderValue::from_str(cookie) {
            headers.insert(reqwest::header::COOKIE, val);
        }

    let mut website = Website::new(&args.url)
        .with_depth(args.max_depth)
        .with_delay((args.delay * 1000.0) as u64)
        .with_user_agent(Some(&args.user_agent))
        .build()
        .unwrap();

    website.configuration.respect_robots_txt = !args.ignore_robots_txt;
    website.configuration.request_timeout = Some(std::time::Duration::from_secs(args.timeout));
    website.configuration.retry = args.retries as u8;

    match args.constraint {
        UrlConstraint::Strict => {
            website.configuration.subdomains = false;
        }
        UrlConstraint::Host => {
            website.configuration.subdomains = true;
        }
    }

    if args.headless {
        let mut intercept_config = spider::features::chrome_common::RequestInterceptConfiguration::default();
        intercept_config.enabled = true;
        website.with_chrome_intercept(intercept_config);
    }

    if !headers.is_empty() {
        website.with_headers(Some(headers.clone()));
    }

    if let Some(exclude) = &args.exclude_regex {
        let exclude_compact: Vec<spider::compact_str::CompactString> = exclude.iter().map(|s| spider::compact_str::CompactString::new(s)).collect();
        website.configuration.with_blacklist_url(Some(exclude_compact));
    }


async fn download_root_files(client: &Client, base_url: &Url, output_dir: &Path) {
    let files = ["robots.txt", "llms.txt", "llms-full.txt"];
    let mut futures = Vec::new();
    
    for file in files {
        if let Ok(url) = base_url.join(file) {
            let client = client.clone();
            let url_clone = url.clone();
            let output_path = output_dir.join(file);
            
            futures.push(tokio::spawn(async move {
                if let Ok(resp) = client.get(url_clone.clone()).timeout(std::time::Duration::from_secs(30)).send().await
                    && resp.status().is_success()
                        && let Ok(bytes) = resp.bytes().await {
                            if let Some(parent) = output_path.parent() {
                                let _ = tokio::fs::create_dir_all(parent).await;
                            }
                            let _ = tokio::fs::write(&output_path, bytes).await;
                            tracing::info!("Downloaded root file: /{}", file);
                        }
            }));
        }
    }
    
    for fut in futures {
        let _ = fut.await;
    }
}

async fn process_html_page(
    mut html: String,
    page_url_str: &str,
    page_url: Url,
    base_url: &Url,
    output_dir: &Path,
    args: Arc<Args>,
    file_counter: Arc<AtomicUsize>,
    semaphore: Arc<Semaphore>,
    downloaded_asset_paths: Arc<DashMap<Url, PathBuf>>,
    client: Client,
    metadata_tx: Option<tokio::sync::mpsc::UnboundedSender<(String, String, String, String)>>,
    browser: Option<Arc<headless_chrome::Browser>>,
    search_index: Option<Arc<DashMap<String, SearchIndexItem>>>,
    pb: ProgressBar,
) {
    pb.set_message(format!("Extracted: {}", page_url_str));
    pb.inc(1);


    if args.extract_metadata {
        let (title, description, keywords) = {
            let document = scraper::Html::parse_document(&html);
            let title_selector = scraper::Selector::parse("title").unwrap();
            let meta_selector = scraper::Selector::parse("meta").unwrap();
            
            let title = document.select(&title_selector).next().map(|e| e.inner_html()).unwrap_or_default();
            let mut description = String::new();
            let mut keywords = String::new();
            
            for meta in document.select(&meta_selector) {
                if let Some(name) = meta.value().attr("name") {
                    if name.eq_ignore_ascii_case("description") {
                        description = meta.value().attr("content").unwrap_or_default().to_string();
                    } else if name.eq_ignore_ascii_case("keywords") {
                        keywords = meta.value().attr("content").unwrap_or_default().to_string();
                    }
                }
            }
            (title, description, keywords)
        };
        
        if let Some(tx) = &metadata_tx {
            let _ = tx.send((page_url_str.to_string(), title, description, keywords));
        }
    }

    if let Some(selector_str) = &args.css_selector
        && let Ok(selector) = scraper::Selector::parse(selector_str) {
            let document = scraper::Html::parse_document(&html);
            let mut extracted = String::new();
            for element in document.select(&selector) {
                extracted.push_str(&element.html());
            }
            if extracted.is_empty() {
                tracing::warn!("CSS selector '{}' found no matches on {}. Skipping page.", selector_str, page_url_str);
                return;
            } else {
                html = extracted;
            }
        }

    let (html_without_base, effective_page_url) = handle_base_tag(&html, &page_url);

    if let Some(local_path) = url_to_local_path(&page_url, base_url, output_dir, args.replace_special_chars, None) {
        if let Some(parent) = local_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        
        let assets = extract_assets(&html_without_base, &effective_page_url);
        let mut asset_paths = HashMap::new();
        
        let mut fetch_futures = Vec::new();
        let mut unique_assets = std::collections::HashSet::new();
        for a in assets { unique_assets.insert(a); }
        for asset_url in unique_assets {
            if !is_same_or_subdomain(asset_url.host_str(), base_url.host_str()) { continue; }
            let client_c = client.clone();
            let args_c = Arc::clone(&args);
            let dl_assets_c = Arc::clone(&downloaded_asset_paths);
            let base_url_c = base_url.clone();
            let output_dir_c = output_dir.to_path_buf();
            
            fetch_futures.push(tokio::spawn(async move {
                let mut needs_download = false;
                let mut dl_info = None;
                
                let local_path = if let Some(entry) = dl_assets_c.get(&asset_url) {
                    entry.value().clone()
                } else {
                    let (len, mut ext, supports_ranges) = fetch_asset_info(&asset_url, &client_c, args_c.retries).await.unwrap_or((0, None, false));
                    if asset_url.path().split("/").last().map(|s| s.contains(".")).unwrap_or(false) {
                        ext = None;
                    }
                    if let Some(lp) = url_to_local_path(&asset_url, &base_url_c, &output_dir_c, args_c.replace_special_chars, ext.as_deref()) {
                        dl_assets_c.insert(asset_url.clone(), lp.clone());
                        needs_download = true;
                        dl_info = Some((len, supports_ranges));
                        lp
                    } else {
                        return None;
                    }
                };
                Some((asset_url, local_path, needs_download, dl_info))
            }));
        }
        
        let mut to_download = Vec::new();
        for fut in fetch_futures {
            if let Ok(Some((u, lp, needs_dl, info))) = fut.await {
                asset_paths.insert(u.clone(), lp.clone());
                if needs_dl {
                    to_download.push((u, lp, info.unwrap()));
                }
            }
        }
        
        let rewritten_html = rewrite_html_links(&html_without_base, &effective_page_url, base_url, output_dir, args.replace_special_chars, &asset_paths, args.form_webhook.as_deref());
        
        if args.generate_search {
            if let Some(ref map) = search_index {
                if let Some(item) = build_search_index(&rewritten_html, page_url_str) {
                    map.insert(page_url_str.to_string(), item);
                }
            }
        }

        if args.export_markdown {
            let md = html2md::parse_html(&rewritten_html);
            let mut md_path = local_path.clone();
            md_path.set_extension("md");
            if let Err(e) = tokio::fs::write(&md_path, md).await {
                tracing::error!("Failed to save markdown to {}: {}", md_path.display(), e);
            }
        }

        if args.dry_run {
            tracing::info!("Dry run: skipping save of HTML {} to {}", page_url_str, local_path.display());
            file_counter.fetch_add(1, Ordering::SeqCst);
        } else {
            if let Err(e) = tokio::fs::write(&local_path, rewritten_html).await {
                tracing::error!("Failed to save html to {}: {}", local_path.display(), e);
            } else {
                file_counter.fetch_add(1, Ordering::SeqCst);
                
                if args.export_pdf
                    && let Some(b) = &browser
                        && let Ok(tab) = b.new_tab() {
                            let absolute_path = std::fs::canonicalize(&local_path).unwrap_or_else(|_| local_path.clone());
                            let file_url = format!("file://{}", absolute_path.display());
                            if let Ok(_) = tab.navigate_to(&file_url) {
                                let _ = tab.wait_until_navigated();
                                if let Ok(pdf_data) = tab.print_to_pdf(None) {
                                    let mut pdf_path = local_path.clone();
                                    pdf_path.set_extension("pdf");
                                    let _ = tokio::fs::write(&pdf_path, pdf_data).await;
                                }
                            }
                        }
            }
        }

        for (u, lp, (l, sr)) in to_download {
            let client_clone = client.clone();
            let base_url_clone = base_url.clone();
            let output_dir_clone = output_dir.to_path_buf();
            let args_clone = Arc::clone(&args);
            let fc = Arc::clone(&file_counter);
            let sem_clone = Arc::clone(&semaphore);
            let dl_assets_clone = Arc::clone(&downloaded_asset_paths);
            
            tokio::spawn(async move {
                if let Err(e) = download_asset_recursive(
                    u.clone(), 
                    lp, l, sr,
                    base_url_clone, 
                    output_dir_clone, 
                    client_clone, 
                    args_clone,
                    fc,
                    sem_clone,
                    dl_assets_clone
                ).await {
                    tracing::error!("Error downloading asset {}: {}", u, e);
                }
            });
        }
    }
}

    let mut rx = website.subscribe(100_000);
    let mut client_builder = Client::builder()
        .user_agent(&args.user_agent)
        .timeout(std::time::Duration::from_secs(args.timeout));
        
    if !headers.is_empty() {
        client_builder = client_builder.default_headers(headers.clone());
    }
    
    let client = client_builder.build()?;
    
    let downloaded_asset_paths = Arc::new(DashMap::new());
    let file_counter = Arc::new(AtomicUsize::new(0));
    let semaphore = Arc::new(Semaphore::new(args.connections));
    let search_index_map: Option<Arc<DashMap<String, SearchIndexItem>>> = if args.generate_search {
        Some(Arc::new(DashMap::new()))
    } else {
        None
    };

    // Download common root files before crawling
    if !args.dry_run {
        download_root_files(&client, &base_url, &output_dir).await;
    }

    let mut browser: Option<Arc<headless_chrome::Browser>> = None;
    if args.headless || args.export_pdf {
        if let Ok(b) = headless_chrome::Browser::default() {
            browser = Some(Arc::new(b));
        } else {
            tracing::warn!("Failed to launch headless Chrome. Ensure Chrome is installed.");
        }
    }
    
    let mut metadata_tx = None;
    if args.extract_metadata {
        let meta_path = output_dir.join("metadata.csv");
        if let Some(parent) = meta_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(mut w) = csv::Writer::from_path(&meta_path) {
            let _ = w.write_record(&["url", "title", "description", "keywords"]);
            let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, String, String, String)>();
            metadata_tx = Some(tx);
            
            tokio::spawn(async move {
                while let Some((url, title, desc, keywords)) = rx.recv().await {
                    let _ = w.write_record(&[url, title, desc, keywords]);
                    let _ = w.flush();
                }
            });
        } else {
            tracing::error!("Failed to create metadata.csv file.");
        }
    }

    let scan_sitemaps = !args.no_sitemaps;
    let sitemap_timeout_secs = args.sitemap_timeout;
    let base_url_clone = base_url.clone();
    let _client_clone = client.clone();
    
    // Shared checklist for URLs discovered via sitemap
    let sitemap_checklist = Arc::new(DashSet::new());
    let sitemap_checklist_clone = sitemap_checklist.clone();
    
    let main_pb_clone = main_pb.clone();
    let crawler_handle = tokio::spawn(async move {
        if scan_sitemaps {
            tracing::info!("Probing for common sitemaps (timeout: {}s)...", sitemap_timeout_secs);
            let sitemap_timeout_dur = std::time::Duration::from_secs(sitemap_timeout_secs);
            let sitemap_res = tokio::time::timeout(sitemap_timeout_dur, website.crawl_sitemap()).await;
            
            if sitemap_res.is_err() {
                tracing::warn!("Sitemap probe timed out after {}s. Falling back directly to HTML link spidering.", sitemap_timeout_secs);
            } else {
                website.persist_links();
                
                let links = website.get_links();
                for link in links {
                    if let Ok(u) = Url::parse(link.as_ref()) {
                        if is_same_or_subdomain(u.host_str(), base_url_clone.host_str()) {
                            sitemap_checklist_clone.insert(u);
                        }
                    }
                }
                
                let count = sitemap_checklist_clone.len();
                tracing::info!("Sitemap crawl discovered {} unique URLs.", count);
                main_pb_clone.set_length(count as u64);
            }
            
            website.crawl().await;
        } else {
            website.crawl().await;
        }
    });

    loop {
        let page = match rx.recv().await {
            Ok(p) => p,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Channel lagged behind by {} pages. Some pages were skipped. Consider lowering concurrency.", n);
                continue;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                break;
            }
        };


        if args.max_files > 0 && file_counter.load(Ordering::SeqCst) >= args.max_files {
            break;
        }

        let page_url_str = page.get_url();
        let page_url = match Url::parse(page_url_str) {
            Ok(u) => {
                sitemap_checklist.remove(&u);
                u
            },
            Err(_) => continue,
        };

        let status = page.status_code;
        if !status.is_success() && status.as_u16() != 0 
            && !args.download_error_pages {
                tracing::warn!("Skipping error page ({}): {}", status, page_url_str);
                continue;
            }
        
        let html = page.get_html();
        if html.is_empty() {
            continue;
        }
        
        let current_pos = file_counter.load(Ordering::SeqCst) as u64;
        if current_pos >= main_pb.length().unwrap_or(100) {
            main_pb.set_length(current_pos + 1);
        }
        process_html_page(
            html,
            page_url_str,
            page_url,
            &base_url,
            &output_dir,
            Arc::clone(&args),
            Arc::clone(&file_counter),
            Arc::clone(&semaphore),
            Arc::clone(&downloaded_asset_paths),
            client.clone(),
            metadata_tx.clone(),
            browser.clone(),
            search_index_map.clone(),
            main_pb.clone(),
        ).await;
    }

    if args.max_files > 0 && file_counter.load(Ordering::SeqCst) >= args.max_files {
        crawler_handle.abort();
    }
    let _ = crawler_handle.await;
    
    // Process remaining urls from the checklist
    let remaining_urls: Vec<Url> = if args.max_files > 0 && file_counter.load(Ordering::SeqCst) >= args.max_files {
        Vec::new()
    } else {
        let checklist = sitemap_checklist;
        checklist.iter().map(|k| k.clone()).collect()
    };
    
    if !remaining_urls.is_empty() {
        tracing::info!("Spider finished, but missed {} URLs from the sitemap. Fetching them manually...", remaining_urls.len());
        
        let mut manual_futures = Vec::new();
        for url in remaining_urls {

            if args.max_files > 0 && file_counter.load(Ordering::SeqCst) >= args.max_files {
                break;
            }
            
            let client = client.clone();
            let base_url = base_url.clone();
            let output_dir = output_dir.to_path_buf();
            let args = Arc::clone(&args);
            let file_counter = Arc::clone(&file_counter);
            let semaphore = Arc::clone(&semaphore);
            let downloaded_asset_paths = Arc::clone(&downloaded_asset_paths);
            
            let metadata_tx = metadata_tx.clone();
            let browser = browser.clone();
            let pb = main_pb.clone();
            let search_map_clone = search_index_map.clone();
            
            manual_futures.push(tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                
                let mut html_opt = None;
                
                if args.headless && browser.is_some() {
                    if let Ok(tab) = browser.as_ref().unwrap().new_tab()
                        && tab.navigate_to(url.as_str()).is_ok() && tab.wait_until_navigated().is_ok() {
                            if args.headless_scroll {
                                scroll_page_to_bottom(&tab);
                            }
                            if let Ok(content) = tab.get_content() {
                                html_opt = Some(content);
                            }
                        }
                } else {
                    if let Ok(resp) = client.get(url.clone()).timeout(std::time::Duration::from_secs(30)).send().await
                        && resp.status().is_success()
                            && let Ok(text) = resp.text().await {
                                html_opt = Some(text);
                            }
                }
                
                if let Some(text) = html_opt {
                    let url_str = url.to_string();
                    process_html_page(
                        text,
                        &url_str,
                        url,
                        &base_url,
                        &output_dir,
                        args,
                        file_counter,
                        Arc::clone(&semaphore),
                        downloaded_asset_paths,
                        client,
                        metadata_tx,
                        browser,
                        search_map_clone,
                        pb,
                    ).await;
                }
            }));
        }
        
        for fut in manual_futures {
            let _ = fut.await;
        }
    }
    
    if args.generate_search && let Some(ref map) = search_index_map {
        let items: Vec<SearchIndexItem> = map.iter().map(|kv| kv.value().clone()).collect();
        let search_path = output_dir.join("search_index.json");
        if let Ok(json) = serde_json::to_string_pretty(&items) {
            let _ = tokio::fs::write(&search_path, json).await;
            tracing::info!("Saved search index with {} items to {}", items.len(), search_path.display());
        }
    }

    main_pb.finish_with_message("Done!");
    let elapsed = start_time.elapsed();
    let total_pages = file_counter.load(Ordering::SeqCst);
    
    println!("");
    println!("{}", "========================================".cyan().bold());
    println!("🎉 {} in {}s!", "Backup Complete".green().bold(), elapsed.as_secs());
    println!("📄 Processed {} pages.", total_pages.to_string().cyan());
    println!("{}", "========================================".cyan().bold());
    
    if args.audit {
        let _ = run_fidelity_audit(&output_dir, base_url.host_str());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;
    use std::path::PathBuf;
    use std::collections::HashMap;

    #[test]
    fn test_is_same_or_subdomain() {
        assert!(is_same_or_subdomain(Some("example.com"), Some("example.com")));
        assert!(is_same_or_subdomain(Some("www.example.com"), Some("example.com")));
        assert!(is_same_or_subdomain(Some("example.com"), Some("www.example.com")));
        assert!(is_same_or_subdomain(Some("cdn.example.com"), Some("example.com")));
        assert!(is_same_or_subdomain(Some("cdn.example.com"), Some("www.example.com")));
        assert!(is_same_or_subdomain(Some("media.cdn.example.com"), Some("example.com")));
        assert!(is_same_or_subdomain(Some("fonts.googleapis.com"), Some("example.com")));
        assert!(is_same_or_subdomain(Some("fonts.gstatic.com"), Some("example.com")));

        assert!(!is_same_or_subdomain(Some("otherdomain.com"), Some("example.com")));
        assert!(!is_same_or_subdomain(Some("notexample.com"), Some("example.com")));
        assert!(!is_same_or_subdomain(None, Some("example.com")));
    }

    #[test]
    fn test_rewrite_html_links_unquoted() {
        let html = r#"<a href=/about>About</a> <img src=/logo.png>"#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert_eq!(
            rewritten,
            r#"<a href="about/index.html">About</a> <img src="logo.png">"#
        );
    }

    #[test]
    fn test_rewrite_html_links_quoted() {
        let html = r#"<a href="/about/">About</a> <a href="https://example.com/contact.html">Contact</a>"#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert_eq!(
            rewritten,
            r#"<a href="about/index.html">About</a> <a href="contact.html">Contact</a>"#
        );
    }

    #[test]
    fn test_rewrite_html_links_form_webhook() {
        let html = r#"<form action="/submit-lead" method="POST"><input name="email"></form>"#;
        let page_url = Url::parse("https://example.com/contact/").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            Some("https://hooks.zapier.com/hooks/catch/12345/"),
        );

        assert!(rewritten.contains(r#"action="https://hooks.zapier.com/hooks/catch/12345/""#));
    }

    #[test]
    fn test_rewrite_html_links_subdomain_bug01() {
        let html = r#"<img src="https://cdn.example.com/assets/banner.png"> <script src="https://www.example.com/js/app.js"></script>"#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert_eq!(
            rewritten,
            r#"<img src="assets/banner.png"> <script src="js/app.js"></script>"#
        );
    }

    #[test]
    fn test_extract_and_rewrite_inline_and_tag_style_bug02() {
        let html = r#"
            <html>
            <head>
                <style>
                    body { background: url('https://cdn.example.com/bg.jpg'); }
                    @font-face { font-family: 'Custom'; src: url(/fonts/custom.woff2); }
                </style>
            </head>
            <body>
                <div style="background-image: url('https://example.com/hero.png'); color: red;">Hero</div>
            </body>
            </html>
        "#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let assets = extract_assets(html, &page_url);
        assert!(assets.iter().any(|u| u.as_str() == "https://cdn.example.com/bg.jpg"));
        assert!(assets.iter().any(|u| u.as_str() == "https://example.com/fonts/custom.woff2"));
        assert!(assets.iter().any(|u| u.as_str() == "https://example.com/hero.png"));

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert!(rewritten.contains("url('bg.jpg')"));
        assert!(rewritten.contains("url('fonts/custom.woff2')"));
        assert!(rewritten.contains("url('hero.png')"));
    }

    #[test]
    fn test_protocol_relative_urls_bug03() {
        let html = r#"<img srcset="//cdn.example.com/img-small.jpg 320w, //cdn.example.com/img-large.jpg 800w">"#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let assets = extract_assets(html, &page_url);
        assert!(assets.iter().any(|u| u.as_str() == "https://cdn.example.com/img-small.jpg"));
        assert!(assets.iter().any(|u| u.as_str() == "https://cdn.example.com/img-large.jpg"));

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert_eq!(
            rewritten,
            r#"<img srcset="img-small.jpg 320w, img-large.jpg 800w">"#
        );
    }

    #[test]
    fn test_rewrite_html_links_fragments() {
        let html = r#"<a href="/about/#team">About Team</a> <a href="?sort=desc">Sort</a>"#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert_eq!(
            rewritten,
            r#"<a href="about/index.html#team">About Team</a> <a href="index_sort_desc.html">Sort</a>"#
        );
    }

    #[test]
    fn test_url_to_local_path() {
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");

        let url1 = Url::parse("https://example.com/").unwrap();
        assert_eq!(
            url_to_local_path(&url1, &base_url, &output_dir, false, None).unwrap(),
            PathBuf::from("backup/index.html")
        );

        let url2 = Url::parse("https://example.com/about").unwrap();
        assert_eq!(
            url_to_local_path(&url2, &base_url, &output_dir, false, None).unwrap(),
            PathBuf::from("backup/about/index.html")
        );

        let url3 = Url::parse("https://example.com/about/").unwrap();
        assert_eq!(
            url_to_local_path(&url3, &base_url, &output_dir, false, None).unwrap(),
            PathBuf::from("backup/about/index.html")
        );

        let url4 = Url::parse("https://example.com/image.png").unwrap();
        assert_eq!(
            url_to_local_path(&url4, &base_url, &output_dir, false, None).unwrap(),
            PathBuf::from("backup/image.png")
        );
    }

    #[test]
    fn test_rewrite_html_links_srcset() {
        let html = r#"<img srcset="/img/small.jpg 320w, /img/large.jpg 800w">"#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert_eq!(
            rewritten,
            r#"<img srcset="img/small.jpg 320w, img/large.jpg 800w">"#
        );
    }

    #[test]
    fn test_extract_metadata_logic() {
        let html = r#"<html><head><title>Test Title</title><meta name="description" content="A test description"><meta name="keywords" content="test, rust, spider"></head><body></body></html>"#;
        let document = scraper::Html::parse_document(html);
        let title_selector = scraper::Selector::parse("title").unwrap();
        let meta_selector = scraper::Selector::parse("meta").unwrap();
        
        let title = document.select(&title_selector).next().map(|e| e.inner_html()).unwrap_or_default();
        let mut description = String::new();
        let mut keywords = String::new();
        
        for meta in document.select(&meta_selector) {
            if let Some(name) = meta.value().attr("name") {
                if name.eq_ignore_ascii_case("description") {
                    description = meta.value().attr("content").unwrap_or_default().to_string();
                } else if name.eq_ignore_ascii_case("keywords") {
                    keywords = meta.value().attr("content").unwrap_or_default().to_string();
                }
            }
        }
        
        assert_eq!(title, "Test Title");
        assert_eq!(description, "A test description");
        assert_eq!(keywords, "test, rust, spider");
    }

    #[test]
    fn test_css_selector_logic() {
        let html = r#"<html><body><div class="unwanted">Bad</div><main class="content">Good stuff</main></body></html>"#;
        let selector = scraper::Selector::parse("main.content").unwrap();
        let document = scraper::Html::parse_document(html);
        let mut extracted = String::new();
        for element in document.select(&selector) {
            extracted.push_str(&element.html());
        }
        assert_eq!(extracted, r#"<main class="content">Good stuff</main>"#);
    }

    #[test]
    fn test_markdown_conversion() {
        let html = r#"<h1>Hello World</h1><p>This is a <strong>test</strong> with a <a href="https://example.com">link</a>.</p><ul><li>Item 1</li><li>Item 2</li></ul>"#;
        let md = html2md::parse_html(html);
        assert!(md.contains("Hello World"));
        assert!(md.contains("**test**"));
        assert!(md.contains("[link](https://example.com)"));
        assert!(md.contains("* Item 1"));
    }

    #[test]
    fn test_build_search_index() {
        let html = r#"
            <html>
            <head><title>About Our Law Firm</title><meta name="description" content="Dedicated trial attorneys."></head>
            <body>
                <h1>Trial Lawyers</h1>
                <h2>Proven Results</h2>
                <p>Over $1 Billion recovered for clients.</p>
            </body>
            </html>
        "#;
        let item = build_search_index(html, "https://example.com/about/").unwrap();
        assert_eq!(item.title, "About Our Law Firm");
        assert_eq!(item.description, "Dedicated trial attorneys.");
        assert_eq!(item.headings, vec!["Trial Lawyers", "Proven Results"]);
    }

    #[test]
    fn test_lazy_load_attributes_bug04() {
        let html = r#"
            <img data-src="https://example.com/lazy.jpg" data-lazy-src="/images/lazy2.jpg" data-orig-file="https://cdn.example.com/orig.png">
            <source data-lazy-srcset="https://example.com/s1.jpg 1x, https://example.com/s2.jpg 2x">
            <div data-bg="url('https://cdn.example.com/bg-card.jpg')">Card</div>
            <svg><use xlink:href="https://example.com/icons.svg#chevron"></use></svg>
        "#;
        let page_url = Url::parse("https://example.com/index.html").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let assets = extract_assets(html, &page_url);
        assert!(assets.iter().any(|u| u.as_str() == "https://example.com/lazy.jpg"));
        assert!(assets.iter().any(|u| u.as_str() == "https://example.com/images/lazy2.jpg"));
        assert!(assets.iter().any(|u| u.as_str() == "https://cdn.example.com/orig.png"));
        assert!(assets.iter().any(|u| u.as_str() == "https://example.com/s1.jpg"));
        assert!(assets.iter().any(|u| u.as_str() == "https://cdn.example.com/bg-card.jpg"));
        assert!(assets.iter().any(|u| u.as_str() == "https://example.com/icons.svg#chevron"));

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert!(rewritten.contains(r#"data-src="lazy.jpg""#));
        assert!(rewritten.contains(r#"data-lazy-src="images/lazy2.jpg""#));
        assert!(rewritten.contains(r#"data-orig-file="orig.png""#));
        assert!(rewritten.contains(r#"data-lazy-srcset="s1.jpg 1x, s2.jpg 2x""#));
        assert!(rewritten.contains(r#"data-bg="url('bg-card.jpg')""#));
        assert!(rewritten.contains(r#"xlink:href="icons.svg#chevron""#));
    }

    #[test]
    fn test_deep_nested_relative_path_bug02() {
        let html = r#"<a href="/practice-areas/">Areas</a> <img src="/assets/logo.png"> <a href="/blog/news/">News</a>"#;
        let page_url = Url::parse("https://example.com/blog/posts/article-one/").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let rewritten = rewrite_html_links(
            html,
            &page_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
            None,
        );

        assert_eq!(
            rewritten,
            r#"<a href="../../../practice-areas/index.html">Areas</a> <img src="../../../assets/logo.png"> <a href="../../news/index.html">News</a>"#
        );
    }

    #[test]
    fn test_html_entities_in_css_url() {
        let css = r#"background: url(&quot;https://cdn.example.com/banner.jpg&quot;);"#;
        let css_url = Url::parse("https://example.com/css/main.css").unwrap();
        let base_url = Url::parse("https://example.com").unwrap();
        let output_dir = PathBuf::from("backup");
        let asset_paths = HashMap::new();

        let assets = extract_css_assets(css, &css_url);
        assert_eq!(assets[0].as_str(), "https://cdn.example.com/banner.jpg");

        let rewritten = rewrite_css_links(
            css,
            &css_url,
            &base_url,
            &output_dir,
            false,
            &asset_paths,
        );
        assert_eq!(rewritten, "background: url('../banner.jpg');");
    }

    #[test]
    fn test_preview_server_file_resolution() {
        let temp_dir = tempfile::tempdir().unwrap();
        let base_path = temp_dir.path();
        
        fs::write(base_path.join("index.html"), "<h1>Home</h1>").unwrap();
        fs::create_dir_all(base_path.join("about")).unwrap();
        fs::write(base_path.join("about/index.html"), "<h1>About</h1>").unwrap();
        fs::write(base_path.join("contact.html"), "<h1>Contact</h1>").unwrap();
        fs::write(base_path.join("404.html"), "<h1>404</h1>").unwrap();

        // 1. Root index
        let (p1, s1) = resolve_preview_file(base_path, "", false);
        assert_eq!(s1, 200);
        assert_eq!(p1.unwrap(), base_path.join("index.html"));

        // 2. Clean URL mapping to /about/index.html
        let (p2, s2) = resolve_preview_file(base_path, "about", false);
        assert_eq!(s2, 200);
        assert_eq!(p2.unwrap(), base_path.join("about/index.html"));

        // 3. Clean URL mapping to contact.html
        let (p3, s3) = resolve_preview_file(base_path, "contact", false);
        assert_eq!(s3, 200);
        assert_eq!(p3.unwrap(), base_path.join("contact.html"));

        // 4. Missing page falls back to 404.html with status 404
        let (p4, s4) = resolve_preview_file(base_path, "nonexistent", false);
        assert_eq!(s4, 404);
        assert_eq!(p4.unwrap(), base_path.join("404.html"));

        // 5. SPA mode fallback to index.html with status 200
        let (p5, s5) = resolve_preview_file(base_path, "spa-route", true);
        assert_eq!(s5, 200);
        assert_eq!(p5.unwrap(), base_path.join("index.html"));
    }

    #[test]
    fn test_percent_decode_path() {
        assert_eq!(percent_decode_path("/about%20us/"), "/about us/");
        assert_eq!(percent_decode_path("/file%2Dname.pdf"), "/file-name.pdf");
        assert_eq!(percent_decode_path("/normal/path"), "/normal/path");
    }

    #[test]
    fn test_fidelity_audit_perfect_score() {
        let temp_dir = tempfile::tempdir().unwrap();
        let p = temp_dir.path();
        
        fs::write(p.join("index.html"), r#"<!DOCTYPE html><html><body><a href="about.html">About</a><img src="logo.png"></body></html>"#).unwrap();
        fs::write(p.join("about.html"), r#"<!DOCTYPE html><html><body><a href="index.html">Home</a></body></html>"#).unwrap();
        fs::write(p.join("logo.png"), b"fake_png").unwrap();

        let report = run_fidelity_audit(p, Some("example.com")).unwrap();
        assert_eq!(report.origin_leaks, 0);
        assert_eq!(report.root_relative_breaks, 0);
        assert_eq!(report.broken_local_links, 0);
        assert_eq!(report.missing_assets, 0);
        assert_eq!(report.fidelity_score, 100.0);
        assert_eq!(report.grade, "A+");
        assert!(p.join("audit_report.md").exists());
        assert!(p.join("audit_report.json").exists());
    }

    #[test]
    fn test_fidelity_audit_detected_leaks_and_broken() {
        let temp_dir = tempfile::tempdir().unwrap();
        let p = temp_dir.path();
        
        // Page contains:
        // 1 origin leak: https://example.com/unrewritten
        // 1 root relative: /broken-root
        // 1 broken local link: missing.html
        // 1 missing asset: missing.jpg
        fs::write(
            p.join("index.html"),
            r#"<a href="https://example.com/unrewritten">Leak</a> <a href="/broken-root">Root</a> <a href="missing.html">404</a> <img src="missing.jpg">"#
        ).unwrap();

        let report = run_fidelity_audit(p, Some("example.com")).unwrap();
        assert_eq!(report.origin_leaks, 1);
        assert_eq!(report.root_relative_breaks, 1);
        assert_eq!(report.broken_local_links, 1);
        assert_eq!(report.missing_assets, 1);
        assert!(report.fidelity_score < 100.0);
    }

    #[test]
    fn test_parse_facebook_html() {
        let html = r#"
            <div role="feed">
                <div>
                    <div data-ad-preview="message">Exciting announcement! Our new website is live.</div>
                    <img src="https://scontent.xx.fbcdn.net/v/t39.30808-6/banner.jpg" class="scaledImageFitWidth">
                    <a href="https://www.facebook.com/Nike/posts/101589321">Post Link</a>
                </div>
                <div>
                    <div data-ad-preview="message">Just Do It. Summer collection is here.</div>
                    <img src="https://scontent.xx.fbcdn.net/v/t39.30808-6/shoe.jpg">
                </div>
            </div>
        "#;
        let posts = parse_facebook_html(html, "https://www.facebook.com/Nike");
        assert_eq!(posts.len(), 2);
        assert!(posts[0].text.contains("Exciting announcement!"));
        assert_eq!(posts[0].media_urls.len(), 1);
        assert_eq!(posts[0].media_urls[0], "https://scontent.xx.fbcdn.net/v/t39.30808-6/banner.jpg");
        assert!(posts[1].text.contains("Just Do It"));
    }

    #[test]
    fn test_generate_facebook_static_feed() {
        let archive = FacebookPageArchive {
            page_name: "Nike".to_string(),
            page_url: "https://www.facebook.com/Nike".to_string(),
            archived_at: "2026-08-28T06:00:00Z".to_string(),
            total_posts: 1,
            posts: vec![
                FacebookPost {
                    post_id: "post_1".to_string(),
                    author: "Nike".to_string(),
                    timestamp: "2026-08-28T06:00:00Z".to_string(),
                    text: "Just Do It.".to_string(),
                    likes: 1500,
                    comments_count: 320,
                    shares_count: 85,
                    media_urls: vec!["https://scontent.xx.fbcdn.net/v/t39.30808-6/banner.jpg".to_string()],
                    local_media_paths: vec!["media/post_1_media_1.jpg".to_string()],
                    post_url: "https://www.facebook.com/Nike/posts/1".to_string(),
                }
            ],
        };

        let html = generate_facebook_static_feed(&archive);
        assert!(html.contains("Nike - Facebook Archive"));
        assert!(html.contains("Just Do It."));
        assert!(html.contains("media/post_1_media_1.jpg"));
        assert!(html.contains("1500 Likes"));
    }
}

