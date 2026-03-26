#!/usr/bin/env node
/**
 * Downloads the correct sandcastle binary for the current platform.
 * Runs as npm postinstall hook.
 *
 * Falls back gracefully if:
 * - The binary is already in PATH
 * - No GitHub release exists yet
 * - Network is unavailable
 */

const { execSync } = require("child_process");
const { createWriteStream, mkdirSync, chmodSync, existsSync } = require("fs");
const { join } = require("path");
const https = require("https");

// Must match the repo where GitHub Releases are published
const REPO = "tylergibbs1/sandcastle";
const BIN_DIR = join(__dirname, "..", ".sandcastle-bin");
const BIN_PATH = join(BIN_DIR, process.platform === "win32" ? "sandcastle.exe" : "sandcastle");

function getPlatformBinary() {
  const arch = process.arch === "arm64" ? "arm64" : "x64";
  const platform = { darwin: "macos", linux: "linux" }[process.platform];
  if (!platform) return null; // Windows and other platforms: no pre-built binary
  return `sandcastle-${platform}-${arch}`;
}

function isAlreadyInstalled() {
  try {
    execSync("sandcastle info", { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function fetch(url) {
  return new Promise((resolve, reject) => {
    https
      .get(url, { headers: { "User-Agent": "sandcastle-npm" } }, (res) => {
        if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
          return fetch(res.headers.location).then(resolve, reject);
        }
        if (res.statusCode !== 200) {
          return reject(new Error(`HTTP ${res.statusCode}`));
        }
        resolve(res);
      })
      .on("error", reject);
  });
}

async function downloadBinary() {
  const name = getPlatformBinary();
  if (!name) {
    console.log("sandcastle: unsupported platform for automatic binary download.");
    console.log("sandcastle: install manually from source or GitHub releases.");
    return;
  }

  if (isAlreadyInstalled()) {
    console.log("sandcastle: binary already in PATH, skipping download");
    return;
  }

  // Get latest release
  let releaseUrl;
  try {
    const res = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`);
    const chunks = [];
    for await (const chunk of res) chunks.push(chunk);
    const release = JSON.parse(Buffer.concat(chunks).toString());
    const asset = release.assets?.find((a) => a.name === `${name}.tar.gz`);
    if (!asset) {
      console.log(`sandcastle: no pre-built binary for ${name} in latest release`);
      console.log("sandcastle: install Rust and build from source: https://github.com/" + REPO);
      return;
    }
    releaseUrl = asset.browser_download_url;
  } catch {
    console.log("sandcastle: could not fetch release info");
    console.log("sandcastle: install manually: https://github.com/" + REPO);
    return;
  }

  // Download and extract
  try {
    console.log(`sandcastle: downloading ${name}...`);
    mkdirSync(BIN_DIR, { recursive: true });

    const tarPath = join(BIN_DIR, `${name}.tar.gz`);
    const res = await fetch(releaseUrl);
    const file = createWriteStream(tarPath);
    await new Promise((resolve, reject) => {
      res.pipe(file);
      file.on("finish", resolve);
      file.on("error", reject);
    });

    execSync(`tar xzf "${tarPath}" -C "${BIN_DIR}"`, { stdio: "ignore" });
    const extracted = join(BIN_DIR, name);
    if (existsSync(extracted)) {
      const { renameSync } = require("fs");
      renameSync(extracted, BIN_PATH);
      chmodSync(BIN_PATH, 0o755);
    }

    require("fs").unlinkSync(tarPath);
    console.log(`sandcastle: installed to ${BIN_PATH}`);
  } catch {
    console.log("sandcastle: binary download failed");
    console.log("sandcastle: install manually: https://github.com/" + REPO);
  }
}

downloadBinary().catch(() => {
  // Never fail the npm install
});
