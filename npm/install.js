const fs = require('fs');
const path = require('path');
const https = require('https');
const os = require('os');
const { execSync } = require('child_process');

// Determine OS and Architecture
const platform = os.platform();
const arch = os.arch();

const REPO = 'frontlanestudio/frontlane-static';
// We will pull the version from package.json
const VERSION = require('./package.json').version;

let assetName = 'frontlane-static';

if (platform === 'win32') {
  if (arch !== 'x64') throw new Error('Unsupported architecture on Windows: ' + arch);
  assetName = 'frontlane-static-win32-x64.exe';
} else if (platform === 'darwin') {
  if (arch === 'x64') assetName = 'frontlane-static-darwin-x64';
  else if (arch === 'arm64') assetName = 'frontlane-static-darwin-arm64';
  else throw new Error('Unsupported architecture on macOS: ' + arch);
} else if (platform === 'linux') {
  if (arch !== 'x64') throw new Error('Unsupported architecture on Linux: ' + arch);
  assetName = 'frontlane-static-linux-x64';
} else {
  throw new Error('Unsupported platform: ' + platform);
}

const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${assetName}`;

const binDir = path.join(__dirname, 'bin');
const binaryPath = path.join(binDir, platform === 'win32' ? 'frontlane-static.exe' : 'frontlane-static');

if (!fs.existsSync(binDir)) {
  fs.mkdirSync(binDir, { recursive: true });
}

console.log(`Downloading frontlane-static v${VERSION} for ${platform} ${arch}...`);
console.log(`URL: ${url}`);

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    https.get(url, (response) => {
      if (response.statusCode === 302 || response.statusCode === 301) {
        // Handle redirect
        download(response.headers.location, dest).then(resolve).catch(reject);
      } else if (response.statusCode === 200) {
        response.pipe(file);
        file.on('finish', () => {
          file.close();
          resolve();
        });
      } else {
        reject(new Error(`Failed to download: ${response.statusCode} ${response.statusMessage}`));
      }
    }).on('error', (err) => {
      fs.unlink(dest, () => reject(err));
    });
  });
}

download(url, binaryPath)
  .then(() => {
    // Make executable on Unix
    if (platform !== 'win32') {
      execSync(`chmod +x "${binaryPath}"`);
    }
    console.log('Successfully downloaded and installed frontlane-static binary.');
  })
  .catch((err) => {
    console.error('Error downloading binary:', err.message);
    process.exit(1);
  });
