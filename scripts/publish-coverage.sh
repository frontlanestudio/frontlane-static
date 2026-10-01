#!/usr/bin/env bash
# ==============================================================================
# Publish Code Coverage to Qlty Cloud
# Supports both local CLI execution and CI environments.
# ==============================================================================

set -euo pipefail

TOKEN="${QLTY_COVERAGE_TOKEN:-qltcp_yfYGAJQP2i9DPRGA}"
COMMIT_SHA="${GITHUB_SHA:-$(git rev-parse HEAD)}"
BRANCH="${GITHUB_REF_NAME:-$(git branch --show-current 2>/dev/null || echo "main")}"
BUILD_ID="${GITHUB_RUN_ID:-local-$(date +%s)}"

if [ ! -f "lcov.info" ]; then
  echo "» Generating lcov.info coverage report..."
  cargo llvm-cov --all-targets --lcov --output-path lcov.info
fi

echo "» Publishing code coverage to Qlty Cloud..."
qlty coverage publish lcov.info \
  --token "$TOKEN" \
  --override-commit-sha "$COMMIT_SHA" \
  --override-branch "$BRANCH" \
  --override-build-id "$BUILD_ID"

echo "✅ Qlty code coverage publishing complete."
