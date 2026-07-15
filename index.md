---
layout: default
title: "Frontlane Static - Documentation"
---

<section class="hero container">
  <div class="hero-glow-1"></div>
  <div class="hero-glow-2"></div>
  
  <div class="hero-badge reveal">
    <div class="hero-badge-dot"></div>
    v1.0.0 is live
  </div>

  <h1 class="hero-title reveal delay-1">Perfect static clones,<br>blazing fast.</h1>
  
  <p class="hero-subtitle reveal delay-2">
    Turn dynamic enterprise sites into completely offline, hyper-accurate static clones. Bypasses WAFs, renders JS apps natively, and intelligently maps all assets. Built in Rust.
  </p>

  <div class="hero-actions reveal delay-3">
    <a href="#install" class="btn-primary">Get Started</a>
    <a href="https://github.com/frontlanestudio/frontlane-static" class="btn-secondary" target="_blank" rel="noopener noreferrer">View Source</a>
  </div>

  <div class="terminal-wrapper reveal delay-4">
    <div class="terminal-header">
      <div class="terminal-dots">
        <div class="term-dot r"></div>
        <div class="term-dot y"></div>
        <div class="term-dot g"></div>
      </div>
      <div class="terminal-title">~/projects/frontlane-static</div>
    </div>
    <div class="terminal-body">
      <div><span class="term-prompt">➜</span> <span class="term-cmd">frontlane-static</span> <span class="term-arg">"https://example.com"</span> <span class="term-dim">--output ./backup --headless</span></div>
      <div style="margin-top:0.5rem"><span class="term-dim">[INFO]</span> Initializing headless browser engine...</div>
      <div><span class="term-dim">[INFO]</span> Bypassing WAF and parsing sitemap...</div>
      <div><span class="term-dim">[INFO]</span> Found 1,248 assets to archive.</div>
      
      <div class="term-progress">
        <span class="term-warning">Downloading</span>
        <div class="term-bar-wrap">
          <div class="term-bar-fill"></div>
        </div>
        <span class="term-dim">843 / 1,248</span>
      </div>
      
      <div style="margin-top:1rem; opacity:0; animation: fadeIn 4s infinite 3s;"><span class="term-success">✔</span> Archive complete! Saved to ./backup in 1.4s</div>
    </div>
  </div>
</section>

<section class="features-section container">
  <div class="section-header reveal">
    <h2 class="hero-title" style="font-size: 3rem;">Engineered for scale.</h2>
    <p class="hero-subtitle" style="margin-bottom:0;">Everything you need to archive modern, complex web applications perfectly.</p>
  </div>

  <div class="bento-grid">
    <!-- Card 1 -->
    <div class="bento-card bento-wide reveal">
      <div class="bento-icon">⚡️</div>
      <h3>Hyper-Concurrent Rust Engine</h3>
      <p>Built on Tokio and DashMap, the spider concurrently downloads and parses assets across thousands of connections without blocking, ensuring maximum network saturation.</p>
    </div>
    
    <!-- Card 2 -->
    <div class="bento-card reveal delay-1">
      <div class="bento-icon">🧩</div>
      <h3>Headless JS Rendering</h3>
      <p>Natively spins up headless Chromium to execute React, Vue, and Angular payloads before saving the DOM, capturing the true visual state of the site.</p>
    </div>

    <!-- Card 3 -->
    <div class="bento-card reveal delay-2">
      <div class="bento-icon">🛡️</div>
      <h3>WAF Evasion</h3>
      <p>Intelligently inherits sitemap parsers and spoofs User-Agents to glide through Cloudflare and enterprise CDNs completely undetected.</p>
    </div>

    <!-- Card 4 -->
    <div class="bento-card reveal">
      <div class="bento-icon">🔗</div>
      <h3>Deep Asset Rewriting</h3>
      <p>Scans HTML and iteratively dives deep into CSS files to accurately resolve and perfectly remap `url()` paths, so your offline clone never breaks.</p>
    </div>

    <!-- Card 5 -->
    <div class="bento-card bento-wide reveal delay-1">
      <div class="bento-icon">⏱️</div>
      <h3>Smart Progress ETA</h3>
      <p>Beautiful, terminal-native CLI output that predicts exactly how long your archive will take based on active sitemap discovery rates and real-time bandwidth metrics.</p>
    </div>
  </div>
</section>

<section class="install-section container reveal" id="install">
  <div class="section-header">
    <h2 class="hero-title" style="font-size: 2.5rem;">Quick Install</h2>
    <p class="hero-subtitle">Run it instantly without installing anything, or install it globally.</p>
  </div>

  <div class="install-grid">
    <div class="install-card">
      <h3>NPM / NPX</h3>
      <p>Run instantly via npx (requires Node.js):</p>
      <pre><code>npx @frontlanestudio/frontlane-static "https://example.com" --output ./backup</code></pre>
      <p style="margin-top:1rem; font-size:0.9em;">Or install globally: <code>npm install -g @frontlanestudio/frontlane-static</code></p>
    </div>

    <div class="install-card">
      <h3>Bun</h3>
      <p>Run instantly via bun (blazing fast):</p>
      <pre><code>bunx @frontlanestudio/frontlane-static "https://example.com" --output ./backup</code></pre>
      <p style="margin-top:1rem; font-size:0.9em;">Or install globally: <code>bun add -g @frontlanestudio/frontlane-static</code></p>
    </div>

    <div class="install-card install-wide">
      <h3>Cargo (Rust Native)</h3>
      <p>Compile and install natively from crates.io (requires Rust):</p>
      <pre><code>cargo install frontlane-static</code></pre>
      <p style="margin-top:1rem; font-size:0.9em;">Then run: <code>frontlane-static "https://example.com" --output ./backup</code></p>
    </div>
  </div>
</section>

<style>
@keyframes fadeIn {
  0% { opacity: 0; }
  100% { opacity: 1; }
}
</style>
