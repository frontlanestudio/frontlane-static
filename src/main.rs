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
    /// URL of the website to backup
    url: String,

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
    let url_host = url.host_str().unwrap_or("");
    let base_host = base_url.host_str().unwrap_or("");
    
    // Check if it's the same host OR a subdomain
    let is_valid_host = url_host == base_host || url_host.ends_with(&format!(".{}", base_host));
    
    if !is_valid_host {
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
        
        let final_ext = force_extension.map(|s| s.to_string()).or(orig_ext).unwrap_or_else(|| "html".to_string());
        
        let new_filename = format!("{}{}.{}", stem, query_suffix, final_ext);
        local_path.push(new_filename);
    }
    Some(local_path)
}

fn extract_assets(html_content: &str, page_url: &Url) -> Vec<Url> {
    let mut assets = Vec::new();
    let document = Html::parse_document(html_content);
    
    let selectors = vec![
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
        ("meta[property='og:image'][content]", "content"),
        ("meta[name='twitter:image'][content]", "content"),
        ("link[rel='icon'][href]", "href"),
        ("link[rel='shortcut icon'][href]", "href"),
        ("link[rel='apple-touch-icon'][href]", "href"),
    ];

    for (sel_str, attr) in selectors {
        if let Ok(selector) = Selector::parse(sel_str) {
            for element in document.select(&selector) {
                if let Some(val) = element.value().attr(attr)
                    && let Ok(url) = page_url.join(val) {
                        assets.push(url);
                    }
            }
        }
    }

    let srcset_selectors = ["img[srcset]", "source[srcset]"];
    for sel_str in srcset_selectors {
        if let Ok(selector) = Selector::parse(sel_str) {
            for element in document.select(&selector) {
                if let Some(val) = element.value().attr("srcset") {
                    for part in val.split(',') {
                        let trimmed = part.trim();
                        let url_str = trimmed.split_whitespace().next().unwrap_or("");
                        if !url_str.is_empty()
                            && let Ok(url) = page_url.join(url_str) {
                                assets.push(url);
                            }
                    }
                }
            }
        }
    }

    assets
}

fn extract_css_assets(css_content: &str, css_url: &Url) -> Vec<Url> {
    lazy_static::lazy_static! {
        static ref URL_RE: Regex = Regex::new(r#"url\(['"]?([^'"\)]+)['"]?\)"#).unwrap();
    }
    
    let mut assets = Vec::new();
    for cap in URL_RE.captures_iter(css_content) {
        let link = &cap[1];
        if !link.starts_with("data:")
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
    asset_paths: &HashMap<Url, PathBuf>
) -> String {
    lazy_static::lazy_static! {
        static ref RE: Regex = Regex::new(r#"(?i)(href|src|action|data|poster|srcset)\s*=\s*(?:["']([^"']+)["']|([^\s>]+))"#).unwrap();
    }
    
    let current_local_path = match url_to_local_path(page_url, base_url, output_dir, replace_special, None) {
        Some(p) => p,
        None => return html.to_string(),
    };
    
    let current_dir = current_local_path.parent().unwrap_or(output_dir);

    RE.replace_all(html, |caps: &regex::Captures| {
        let attr = &caps[1];
        let link_or_srcset = caps.get(2).or_else(|| caps.get(3)).map(|m| m.as_str()).unwrap_or("");
        
        if attr.eq_ignore_ascii_case("srcset") {
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
                        && target_url.host_str() == base_url.host_str() {
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
        
        let link = link_or_srcset;
        
        if link.starts_with("data:") || link.starts_with("mailto:") || link.starts_with("tel:") {
            return format!("{}=\"{}\"", attr, link);
        }
        
        if let Ok(target_url) = page_url.join(link)
            && target_url.host_str() == base_url.host_str() {
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
        static ref URL_RE: Regex = Regex::new(r#"url\(['"]?([^'"\)]+)['"]?\)"#).unwrap();
    }
    
    let current_local_path = match url_to_local_path(css_url, base_url, output_dir, replace_special, None) {
        Some(p) => p,
        None => return css.to_string(),
    };
    
    let current_dir = current_local_path.parent().unwrap_or(output_dir);

    URL_RE.replace_all(css, |caps: &regex::Captures| {
        let link = &caps[1];
        
        if link.starts_with("data:") {
            return format!("url('{}')", link);
        }
        
        if let Ok(target_url) = css_url.join(link)
            && target_url.host_str() == base_url.host_str() {
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
        
        format!("url('{}')", link)
    }).into_owned()
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
                        if inner_url.host_str() != base_url.host_str() { continue; }
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

#[tokio::main]
async fn main() -> Result<()> {
    let start_time = std::time::Instant::now();
    let args = Arc::new(Args::parse());

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
            if asset_url.host_str() != base_url.host_str() { continue; }
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
        
        let rewritten_html = rewrite_html_links(&html_without_base, &effective_page_url, base_url, output_dir, args.replace_special_chars, &asset_paths);
        
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
    let base_url_clone = base_url.clone();
    let _client_clone = client.clone();
    
    // Shared checklist for URLs discovered via sitemap
    let sitemap_checklist = Arc::new(DashSet::new());
    let sitemap_checklist_clone = sitemap_checklist.clone();
    
    let main_pb_clone = main_pb.clone();
    let crawler_handle = tokio::spawn(async move {
        if scan_sitemaps {
            tracing::info!("Probing for common sitemaps...");
            website.crawl_sitemap().await;
            website.persist_links();
            
            let links = website.get_links();
            for link in links {
                if let Ok(u) = Url::parse(link.as_ref()) {
                    if u.host_str() == base_url_clone.host_str() || u.host_str().unwrap_or("").ends_with(&format!(".{}", base_url_clone.host_str().unwrap_or(""))) {
                        sitemap_checklist_clone.insert(u);
                    }
                }
            }
            
            let count = sitemap_checklist_clone.len();
            tracing::info!("Sitemap crawl discovered {} unique URLs.", count);
            main_pb_clone.set_length(count as u64);
            
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
            main_pb.clone(),
        ).await;
    }

    crawler_handle.await?;
    
    // Process remaining urls from the checklist
    let remaining_urls: Vec<Url> = {
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
            
            manual_futures.push(tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();
                
                let mut html_opt = None;
                
                if args.headless && browser.is_some() {
                    if let Ok(tab) = browser.as_ref().unwrap().new_tab()
                        && tab.navigate_to(url.as_str()).is_ok() && tab.wait_until_navigated().is_ok()
                            && let Ok(content) = tab.get_content() {
                                html_opt = Some(content);
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
                        pb,
                    ).await;
                }
            }));
        }
        
        for fut in manual_futures {
            let _ = fut.await;
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
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use url::Url;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use colored::*;
    use std::path::PathBuf;
    use std::collections::HashMap;

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
        );

        assert_eq!(
            rewritten,
            r#"<a href="about/index.html">About</a> <a href="contact.html">Contact</a>"#
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
        );

        assert_eq!(
            rewritten,
            r#"<a href="about/index.html#team">About Team</a> <a href="index.html">Sort</a>"#
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
        );

        assert_eq!(
            rewritten,
            r#"<img srcset="img/small.jpg 320w, img/large.jpg 800w">"#
        );
    }
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
