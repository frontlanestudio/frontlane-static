# Frontlane Static 🚀

> Turn your dynamic enterprise sites into blazing-fast, perfect static clones.

[![Build Status](https://img.shields.io/badge/build-passing-brightgreen)](#) [![License](https://img.shields.io/badge/license-MIT-blue)](#) [![Rust](https://img.shields.io/badge/rust-1.70%2B-orange)](#)

**Frontlane Static** is an incredibly fast, highly concurrent static site crawler and archiver built in Rust. It was designed from the ground up to handle enterprise-level crawling, bypassing complex WAFs (like Cloudflare), rendering heavy JavaScript applications via a headless browser, and cleanly rewriting all internal CSS/HTML links so that your offline clone works perfectly.

Created by [Frontlane Studio](https://frontlane.com).

## 🌟 Key Features

*   **1:1 Asset Resolution:** Accurately downloads and perfectly maps every HTML, CSS, JavaScript, Image, Font, and Video asset.
*   **Deep CSS Link Rewriting:** Iteratively resolves `url()` paths inside stylesheets, no matter how deeply nested they are.
*   **Enterprise WAF Bypassing:** Inherits native sitemap parsers and spoofed User-Agents to glide through Cloudflare and other CDNs without getting blocked.
*   **JavaScript Rendering (Headless Chrome):** Natively spins up a headless Chromium instance to fully render React/Vue/Angular sites before downloading the DOM.
*   **Highly Concurrent:** Reaps the power of Tokio and `DashMap` to parallelize asset discovery without stack overflows.
*   **Beautiful CLI:** See exactly how long your crawl will take with dynamic ETA progress bars based on your sitemap.

## 📦 Installation

To install **Frontlane Static**, you'll need [Rust and Cargo](https://rustup.rs/) installed on your machine.

```bash
git clone https://github.com/frontlanestudio/frontlane-static.git
cd frontlane-static
cargo build --release
```

The compiled binary will be located at `target/release/frontlane-static`.

## 🚀 Usage

The CLI is extremely simple to use. Provide the target URL and an output directory.

```bash
frontlane-static "https://example.com" --output ./backup
```

### Advanced Options

*   `--max-depth <NUMBER>`: Limit how deep the spider crawls (e.g., `--max-depth 1` for just the root layer).
*   `--headless`: Use a headless Chromium browser to render JavaScript before downloading the HTML.
*   `--retries <NUMBER>`: Number of times to retry a failed asset download (default: 3).
*   `--min-size <BYTES>` / `--max-size <BYTES>`: Only download assets within this size range.

## 🤝 Contributing

We welcome contributions! Please open an issue or submit a Pull Request on our GitHub repository.

## 📄 License

This project is licensed under the MIT License - see the LICENSE file for details.
