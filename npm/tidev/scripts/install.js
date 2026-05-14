/**
 * tidev npm installer
 *
 * Downloads the prebuilt tidev binary from GitHub Releases on postinstall.
 * Caches the binary in bin/downloads/ for subsequent runs.
 */

const fs = require("fs");
const https = require("https");
const http = require("http");
const crypto = require("crypto");
const { URL } = require("url");
const path = require("path");
const { mkdir, chmod, rename, readFile, writeFile, unlink, stat } = fs.promises;
const { createWriteStream } = fs;

const { detectBinaryName, releaseAssetUrl, releaseBinaryDirectory } = require("./artifacts");
const pkg = require("../package.json");

const MAX_ATTEMPTS = 5;
const BASE_BACKOFF_MS = 1000;

// ── Helpers ───────────────────────────────────────────────────────

function resolveVersion() {
  return process.env.TIDEV_VERSION || pkg.tidevBinaryVersion || pkg.version;
}

function resolveRepo() {
  return process.env.TIDEV_GITHUB_REPO || "Eslzzyl/tidev";
}

function isOptionalInstall(argv = process.argv.slice(2)) {
  return argv.includes("--optional");
}

function isInstallContext(context) {
  return context === "install";
}

function isQuiet() {
  if (process.env.TIDEV_QUIET === "1") return true;
  const level = (process.env.npm_config_loglevel || "").toLowerCase();
  return level === "silent" || level === "error";
}

function log(msg) {
  if (!isQuiet()) process.stderr.write(`tidev: ${msg}\n`);
}

class DownloadError extends Error {
  constructor(msg, retryable = true) {
    super(msg);
    this.retryable = retryable;
  }
}

// ── HTTP download with redirect following ─────────────────────────

function httpGet(url, destPath) {
  return new Promise((resolve, reject) => {
    const parsed = new URL(url);
    const client = parsed.protocol === "https:" ? https : http;

    client.get(url, { headers: { "User-Agent": "tidev-installer" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        res.resume();
        resolve(httpGet(new URL(res.headers.location, url).toString(), destPath));
        return;
      }
      if (res.statusCode !== 200) {
        res.resume();
        reject(new DownloadError(`HTTP ${res.statusCode}: ${url}`, res.statusCode < 500));
        return;
      }
      const file = createWriteStream(destPath);
      res.pipe(file);
      file.on("finish", () => {
        file.close();
        resolve();
      });
      file.on("error", (err) => {
        unlink(destPath).catch(() => {});
        reject(new DownloadError(`Write error: ${err.message}`));
      });
      res.on("error", (err) => {
        unlink(destPath).catch(() => {});
        reject(new DownloadError(`Network error: ${err.message}`));
      });
    }).on("error", (err) => {
      reject(new DownloadError(`Request failed: ${err.message}`));
    });
  });
}

// ── Checksum ──────────────────────────────────────────────────────

async function sha256File(filePath) {
  const content = await readFile(filePath);
  return crypto.createHash("sha256").update(content).digest("hex");
}

async function downloadChecksums(version, repo) {
  const url = releaseAssetUrl(`tidev-sha256.txt`, version, repo);
  const chunks = [];
  return new Promise((resolve, reject) => {
    const parsed = new URL(url);
    const client = parsed.protocol === "https:" ? https : http;
    client.get(url, { headers: { "User-Agent": "tidev-installer" } }, (res) => {
      if (res.statusCode !== 200) {
        res.resume();
        reject(new DownloadError(`Failed to fetch checksums: HTTP ${res.statusCode}`, false));
        return;
      }
      res.setEncoding("utf8");
      res.on("data", (chunk) => chunks.push(chunk));
      res.on("end", () => resolve(chunks.join("")));
      res.on("error", (err) => reject(new DownloadError(`Network error: ${err.message}`)));
    }).on("error", (err) => reject(new DownloadError(`Request failed: ${err.message}`)));
  }).then((text) => {
    const map = new Map();
    for (const line of text.split(/\r?\n/)) {
      const m = line.trim().match(/^([a-f0-9]{64})\s+(.+)$/i);
      if (m) map.set(m[2], m[1].toLowerCase());
    }
    return map;
  });
}

async function verifyChecksum(filePath, assetName, checksums) {
  const expected = checksums.get(assetName);
  if (!expected) throw new DownloadError(`Checksum manifest missing entry for ${assetName}`, false);
  const actual = await sha256File(filePath);
  if (actual !== expected) {
    throw new DownloadError(`Checksum mismatch for ${assetName}: expected ${expected}, got ${actual}`, false);
  }
}

// ── Retry ─────────────────────────────────────────────────────────

async function withRetry(label, fn, context) {
  const maxAttempts = isInstallContext(context) && isOptionalInstall() ? 1 : MAX_ATTEMPTS;
  for (let attempt = 1; attempt <= maxAttempts; attempt++) {
    try {
      return await fn();
    } catch (err) {
      if (err.retryable === false || attempt === maxAttempts) throw err;
      const delay = BASE_BACKOFF_MS * Math.pow(2, attempt - 1);
      log(`${label} failed (attempt ${attempt}/${maxAttempts}), retrying in ${delay}ms...`);
      await new Promise((r) => setTimeout(r, delay));
    }
  }
}

// ── Main ──────────────────────────────────────────────────────────

async function getBinaryPath() {
  const version = resolveVersion();
  const repo = resolveRepo();
  const { name: assetName } = detectBinaryName();
  const releaseDir = releaseBinaryDirectory();
  const isWin = process.platform === "win32";
  const targetPath = path.join(releaseDir, isWin ? "tidev.exe" : "tidev");
  const markerPath = `${targetPath}.version`;

  await mkdir(releaseDir, { recursive: true });

  // ── Check if cached binary is up-to-date ──
  try {
    const cachedVersion = (await readFile(markerPath, "utf8")).trim();
    const cachedStat = await stat(targetPath);
    if (cachedVersion === String(version) && cachedStat.isFile()) {
      log(`Binary (v${version}) already installed.`);
      return targetPath;
    }
  } catch {}

  // ── Download ──
  log(`Downloading tidev v${version} (${assetName})...`);

  const checksums = await withRetry("Download checksums", () => downloadChecksums(version, repo));
  const downloadUrl = releaseAssetUrl(assetName, version, repo);
  const tmpPath = `${targetPath}.${Date.now()}.download`;

  await withRetry(`Download ${assetName}`, async () => {
    await httpGet(downloadUrl, tmpPath);
  });

  await verifyChecksum(tmpPath, assetName, checksums);

  if (!isWin) await chmod(tmpPath, 0o755);
  await rename(tmpPath, targetPath);
  await writeFile(markerPath, String(version), "utf8");

  log(`Installed tidev v${version} to ${targetPath}`);
  return targetPath;
}

// ── postinstall entry ─────────────────────────────────────────────

async function run(options = {}) {
  const context = options.context || "runtime";
  if (process.env.TIDEV_DISABLE_INSTALL === "1") return;

  try {
    await getBinaryPath();
  } catch (err) {
    const shouldIgnore = isInstallContext(context) && isOptionalInstall() && err.retryable !== false;
    if (shouldIgnore) {
      log(`Download failed (optional install): ${err.message}`);
      log("Binary will be downloaded on first run.");
      return;
    }
    log(`Installation failed: ${err.message}`);
    process.exitCode = 1;
  }
}

module.exports = { getBinaryPath, run };

if (require.main === module) {
  const context = process.argv.includes("--optional") ? "install" : "runtime";
  run({ context });
}
