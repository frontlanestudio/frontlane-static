# Frontlane-Static Bug & Feature Report (Real-World Test Suite)

**Date**: August 27, 2026
**Sites Evaluated**: 53 Real-World California Law Firm Backups
**Total Links Checked**: 2,479,416
**Average Clone Fidelity**: 74%

---

## 🎯 Executive Defect Summary

| Issue Code | Description | Instances in Backups | Severity | Proposed Fix |
| :--- | :--- | :---: | :---: | :--- |
| **BUG-01** | **Live Origin Link Leakage**: Links to internal subpages retain absolute `https://domain.com/...` or `//domain.com/...` URLs. | **1,323,945** | **CRITICAL** | Normalize protocol-relative URLs & enforce relative rewriting for all same-origin & subdomain `<a href>`. |
| **BUG-02** | **Root-Relative Links (`/path`)**: Unrewritten leading slash links fail when viewing via `file://` or subfolder servers. | **170,840** | **HIGH** | Compute relative path offset (`./` or `../`) based on current page depth in directory tree. |
| **BUG-03** | **Extension Mismatch & Directory Missing**: Links to `about/` fail because files are saved as `about.html` without directory index mapping. | **238,194** | **HIGH** | Standardize target naming or write local link redirect table into downloaded archive. |
| **BUG-04** | **Missing CSS `url(...)` Background Assets**: Hero images & font files embedded in inline `style="..."` or `<style>` tags not downloaded. | **484,103** | **MEDIUM** | Traverse `[style]` attributes and `<style>` blocks with CSS link extraction regex. |
| **FEAT-01** | **Headless Dynamic Scroll & Lazy-Load Trigger**: SPAs and IntersectionObserver images not triggered during static fetch. | — | **FEATURE** | Add `--headless-scroll` option to scroll to bottom and wait for network idle before snapshotting DOM. |
| **FEAT-02** | **Built-in Local Preview Server with SPA Fallback**: Built-in CLI command (`frontlane-static serve <dir>`) with automatic 404 fallback. | — | **FEATURE** | Add lightweight embedded HTTP server for 1-click local browsing with exact path resolution. |

---

## 🔍 Detailed Bug Analysis & Code Pointers

### BUG-01: Absolute URL Leaking to Live Origin
- **Impact**: Clicking internal links in a cloned site unexpectedly opens the live remote website, breaking offline archiving.
- **Root Cause in `src/main.rs`**:
  The link rewriter `rewrite_html_links` uses strict string equality on `target_url.host_str() == base_url.host_str()`. When links omit the protocol (`//domain.com`) or use subdomains (`www.domain.com` vs `domain.com`), they bypass the rewriter and remain absolute.
- **Fix**:
  ```rust
  fn is_same_site(target_host: Option<&str>, base_host: Option<&str>) -> bool {
      match (target_host, base_host) {
          (Some(t), Some(b)) => {
              let t_clean = t.trim_start_matches("www.");
              let b_clean = b.trim_start_matches("www.");
              t_clean == b_clean || t.ends_with(&format!(".{}", b_clean))
          },
          _ => false,
      }
  }
  ```

### BUG-02: Root-Relative Path Resolution for Offline Viewing
- **Impact**: `<a href="/practice-areas/">` attempts to resolve to the filesystem root (`file:///practice-areas/`) when opened directly in a browser.
- **Fix**:
  Always compute relative depth from the current HTML file's directory:
  ```rust
  fn make_relative_path(from_file: &Path, to_target: &Path) -> PathBuf {
      let from_dir = from_file.parent().unwrap_or(Path::new(""));
      diff_paths(to_target, from_dir).unwrap_or_else(|| to_target.to_path_buf())
  }
  ```

---

## 📋 Site-by-Site Fidelity Audits (Sample)


### 🏢 krasneylawcenter.com
- **Fidelity Score**: 97% [Grade: **A**]
- **Pages**: 35 | **Links Checked**: 1403
- **Origin Leaks**: 0 | **Root-Relative Breaks**: 37 | **Broken Targets**: 0
- **Key Defects**: BUG-02: Root-Relative Links (/path) Unrewritten for Local file://, BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 nakaselawfirm.com
- **Fidelity Score**: 56% [Grade: **D**]
- **Pages**: 797 | **Links Checked**: 80615
- **Origin Leaks**: 52450 | **Root-Relative Breaks**: 0 | **Broken Targets**: 22505
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 vassermanlaw.com
- **Fidelity Score**: 79% [Grade: **C**]
- **Pages**: 14 | **Links Checked**: 117
- **Origin Leaks**: 58 | **Root-Relative Breaks**: 0 | **Broken Targets**: 0
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 mylosangelespersonalinjurylawyer.com
- **Fidelity Score**: 92% [Grade: **A**]
- **Pages**: 1 | **Links Checked**: 7
- **Origin Leaks**: 0 | **Root-Relative Breaks**: 0 | **Broken Targets**: 1
- **Key Defects**: BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 solumspacelaw.com
- **Fidelity Score**: 95% [Grade: **A**]
- **Pages**: 1 | **Links Checked**: 0
- **Origin Leaks**: 0 | **Root-Relative Breaks**: 0 | **Broken Targets**: 0
- **Key Defects**: BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 hodesmilman.com
- **Fidelity Score**: 51% [Grade: **D**]
- **Pages**: 1 | **Links Checked**: 73
- **Origin Leaks**: 0 | **Root-Relative Breaks**: 0 | **Broken Targets**: 61
- **Key Defects**: BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 pintas.com
- **Fidelity Score**: 62% [Grade: **D**]
- **Pages**: 2178 | **Links Checked**: 120074
- **Origin Leaks**: 41955 | **Root-Relative Breaks**: 33440 | **Broken Targets**: 12879
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-02: Root-Relative Links (/path) Unrewritten for Local file://, BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 eatorreslegal.com
- **Fidelity Score**: 100% [Grade: **A**]
- **Pages**: 2 | **Links Checked**: 0
- **Origin Leaks**: 0 | **Root-Relative Breaks**: 0 | **Broken Targets**: 0
- **Key Defects**: None


### 🏢 thegllaw.com
- **Fidelity Score**: 75% [Grade: **C**]
- **Pages**: 29 | **Links Checked**: 1138
- **Origin Leaks**: 301 | **Root-Relative Breaks**: 71 | **Broken Targets**: 221
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-02: Root-Relative Links (/path) Unrewritten for Local file://, BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 koohanimlaw.com
- **Fidelity Score**: 80% [Grade: **B**]
- **Pages**: 151 | **Links Checked**: 11352
- **Origin Leaks**: 5258 | **Root-Relative Breaks**: 0 | **Broken Targets**: 102
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 nnwlegal.com
- **Fidelity Score**: 55% [Grade: **D**]
- **Pages**: 107 | **Links Checked**: 12704
- **Origin Leaks**: 9 | **Root-Relative Breaks**: 7253 | **Broken Targets**: 1140
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-02: Root-Relative Links (/path) Unrewritten for Local file://, BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 rosensteinlaw.com
- **Fidelity Score**: 95% [Grade: **A**]
- **Pages**: 33 | **Links Checked**: 5503
- **Origin Leaks**: 0 | **Root-Relative Breaks**: 0 | **Broken Targets**: 382
- **Key Defects**: BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 peeralilaw.com
- **Fidelity Score**: 77% [Grade: **C**]
- **Pages**: 457 | **Links Checked**: 27105
- **Origin Leaks**: 13177 | **Root-Relative Breaks**: 1021 | **Broken Targets**: 701
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-02: Root-Relative Links (/path) Unrewritten for Local file://, BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 marksonpico.com
- **Fidelity Score**: 75% [Grade: **C**]
- **Pages**: 1 | **Links Checked**: 1
- **Origin Leaks**: 0 | **Root-Relative Breaks**: 0 | **Broken Targets**: 0
- **Key Defects**: BUG-04: Missing Local Assets (CSS/JS/Images 404)


### 🏢 aa.law
- **Fidelity Score**: 48% [Grade: **F**]
- **Pages**: 274 | **Links Checked**: 46907
- **Origin Leaks**: 25545 | **Root-Relative Breaks**: 1405 | **Broken Targets**: 16848
- **Key Defects**: BUG-01: Absolute URL Leaking to Live Origin, BUG-02: Root-Relative Links (/path) Unrewritten for Local file://, BUG-03: Broken Local Target Link (Missing .html or directory), BUG-04: Missing Local Assets (CSS/JS/Images 404)

