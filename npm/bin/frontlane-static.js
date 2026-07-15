#!/usr/bin/env node

const { spawnSync } = require('child_process');
const path = require('path');
const os = require('os');
const fs = require('fs');

const platform = os.platform();
const binName = platform === 'win32' ? 'frontlane-static.exe' : 'frontlane-static';
const binPath = path.join(__dirname, binName);

if (!fs.existsSync(binPath)) {
  console.error(`Error: Could not find the frontlane-static binary at ${binPath}`);
  console.error('Please reinstall the package to download the binary.');
  process.exit(1);
}

// Pass all arguments directly to the Rust binary
const args = process.argv.slice(2);

const result = spawnSync(binPath, args, { stdio: 'inherit' });

if (result.error) {
  console.error('Failed to start frontlane-static:', result.error);
  process.exit(1);
}

process.exit(result.status || 0);
