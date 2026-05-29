/**
 * tidev npm installer
 *
 * Downloads the prebuilt tidev binary from GitHub Releases on postinstall.
 * Caches the binary in bin/downloads/ for subsequent runs.
 */

const fs = require("fs");
const https = require("https");
const http = require("http");
const tls = require("tls");
const crypto = require("crypto");
const { URL } = require("url");
const path = require("path");
const net = require("net");
const { PassThrough } = require("stream");
const { mkdir, chmod, rename, readFile, writeFile, unlink, stat } = fs.promises;
const { createWriteStream } = fs;

const { detectBinaryName, releaseAssetUrl, releaseBinaryDirectory } = require("./artifacts");
const pkg = require("../package.json");

const MAX_ATTEMPTS = 5;
const BASE_BACKOFF_MS = 1000;
const REQUEST_TIMEOUT_MS = 30_000;

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

// ── Proxy detection ───────────────────────────────────────────────

function getProxyUrl(targetProtocol) {
  const key = targetProtocol === "https:" ? "https_proxy" : "http_proxy";
  return process.env[key] || process.env[key.toUpperCase()] || null;
}

// ── Low-level raw HTTP request through proxy or direct ────────────

/**
 * Parse an HTTP response (status line + headers) from a buffer string.
 * Returns { httpVersion, statusCode, statusMessage, headers } and consumes
 * the header portion from the buffer.
 */
function parseHttpResponseHeader(data) {
  const headerEnd = data.indexOf("\r\n\r\n");
  if (headerEnd === -1) return null;
  const headerBlock = data.slice(0, headerEnd);
  const lines = headerBlock.split("\r\n");
  const statusLine = lines[0];
  const statusMatch = statusLine.match(/^HTTP\/(\d\.\d)\s+(\d+)\s+(.*)$/);
  if (!statusMatch) throw new DownloadError(`Malformed HTTP response: ${statusLine}`, false);
  const headers = {};
  for (let i = 1; i < lines.length; i++) {
    const colonIdx = lines[i].indexOf(":");
    if (colonIdx !== -1) {
      const key = lines[i].slice(0, colonIdx).toLowerCase().trim();
      const val = lines[i].slice(colonIdx + 1).trim();
      if (headers[key] === undefined) {
        headers[key] = val;
      } else {
        if (Array.isArray(headers[key])) headers[key].push(val);
        else headers[key] = [headers[key], val];
      }
    }
  }
  return {
    httpVersion: statusMatch[1],
    statusCode: parseInt(statusMatch[2], 10),
    statusMessage: statusMatch[3],
    headers,
    headerLength: headerEnd + 4, // include the \r\n\r\n
  };
}

/**
 * Make a raw HTTP GET request, returning the full response body as a string.
 * Supports HTTP proxy via CONNECT tunnel for HTTPS or direct forwarding for HTTP.
 */
function rawHttpGet(url) {
  const MAX_REDIRECTS = 10;
  const MAX_BODY_SIZE = 10 * 1024 * 1024; // 10 MB limit

  async function doRequest(targetUrl, redirectDepth) {
    if (redirectDepth > MAX_REDIRECTS) {
      throw new DownloadError(`Too many redirects: ${url}`, false);
    }

    const parsed = new URL(targetUrl);
    const isHttps = parsed.protocol === "https:";
    const targetPort = parseInt(parsed.port, 10) || (isHttps ? 443 : 80);
    const proxyUrl = getProxyUrl(parsed.protocol);

    let socket;

    if (proxyUrl) {
      // ── Proxy path ──
      const proxyParsed = new URL(proxyUrl);
      const proxyPort = parseInt(proxyParsed.port, 10) || (proxyParsed.protocol === "https:" ? 443 : 8080);

      socket = await new Promise((resolve, reject) => {
        const sock = net.connect(proxyPort, proxyParsed.hostname, () => {
          if (isHttps) {
            // CONNECT tunnel for HTTPS
            sock.write(`CONNECT ${parsed.hostname}:${targetPort} HTTP/1.1\r\nHost: ${parsed.hostname}:${targetPort}\r\n\r\n`);
          } else {
            // Plain HTTP through proxy — skip CONNECT, send request directly
            resolve(sock);
          }
        });
        sock.setTimeout(REQUEST_TIMEOUT_MS, () => {
          sock.destroy();
          reject(new DownloadError(`Proxy connection timed out: ${proxyUrl}`));
        });
        sock.on("error", (err) => reject(new DownloadError(`Proxy connection error: ${err.message}`)));

        if (isHttps) {
          let buf = "";
          sock.on("data", (chunk) => {
            buf += chunk.toString();
            const idx = buf.indexOf("\r\n\r\n");
            if (idx !== -1) {
              const statusLine = buf.split("\r\n")[0];
              const m = statusLine.match(/HTTP\/\d\.\d\s+(\d+)/);
              if (!m || m[1] !== "200") {
                sock.destroy();
                reject(new DownloadError(`Proxy CONNECT failed: ${statusLine}`, false));
                return;
              }
              sock.removeAllListeners("data");
              // Wrap with TLS
              const tlsSocket = tls.connect({ socket: sock, host: parsed.hostname, servername: parsed.hostname });
              tlsSocket.on("secureConnect", () => resolve(tlsSocket));
              tlsSocket.on("error", (err) => reject(new DownloadError(`TLS error through proxy: ${err.message}`)));
            }
          });
        }
      });
    } else {
      // ── Direct path ──
      if (isHttps) {
        socket = await new Promise((resolve, reject) => {
          const sock = tls.connect(targetPort, parsed.hostname, { servername: parsed.hostname }, () => resolve(sock));
          sock.setTimeout(REQUEST_TIMEOUT_MS, () => { sock.destroy(); reject(new DownloadError(`Connection timed out: ${targetUrl}`)); });
          sock.on("error", (err) => reject(new DownloadError(`Connection error: ${err.message}`)));
        });
      } else {
        socket = await new Promise((resolve, reject) => {
          const sock = net.connect(targetPort, parsed.hostname, () => resolve(sock));
          sock.setTimeout(REQUEST_TIMEOUT_MS, () => { sock.destroy(); reject(new DownloadError(`Connection timed out: ${targetUrl}`)); });
          sock.on("error", (err) => reject(new DownloadError(`Connection error: ${err.message}`)));
        });
      }
    }

    // ── Send HTTP GET request ──
    const requestLine = `GET ${parsed.pathname}${parsed.search} HTTP/1.1\r\nHost: ${parsed.hostname}\r\nUser-Agent: tidev-installer\r\nConnection: close\r\n\r\n`;
    socket.write(requestLine);

    // ── Read response ──
    const chunks = [];
    let headerParsed = false;
    let responseInfo = null;
    let bodyStartOffset = 0;
    let expectedBodyLen = -1;
    let bodyComplete = false;

    const bodyStream = new PassThrough();

    const resultPromise = new Promise((resolve, reject) => {
      function tryResolve() {
        if (bodyComplete) return;

        const allData = Buffer.concat(chunks);

        if (!headerParsed) {
          const info = parseHttpResponseHeader(allData.toString("utf8"));
          if (!info) return; // not enough data yet

          headerParsed = true;
          responseInfo = info;
          bodyStartOffset = info.headerLength;

          // Handle redirect — no body needed
          if (info.statusCode >= 300 && info.statusCode < 400 && info.headers.location) {
            bodyComplete = true;
            const redirectUrl = new URL(info.headers.location, targetUrl).toString();
            socket.destroy();
            resolve(doRequest(redirectUrl, redirectDepth + 1));
            return;
          }

          // Determine expected body length from Content-Length header
          const cl = info.headers["content-length"];
          expectedBodyLen = cl ? parseInt(cl, 10) : -1;
        }

        // We have headers — check if we have the full body
        const receivedBodyLen = allData.length - bodyStartOffset;
        if (expectedBodyLen >= 0) {
          if (receivedBodyLen >= expectedBodyLen) {
            bodyComplete = true;
            const bodyBuf = allData.slice(bodyStartOffset, bodyStartOffset + expectedBodyLen);
            bodyStream.end(bodyBuf);
            resolve({
              statusCode: responseInfo.statusCode,
              headers: responseInfo.headers,
              bodyStream,
            });
          }
        }
        // No Content-Length: wait for socket end (handled in socket.on('end'))
      }

      socket.on("data", (chunk) => {
        chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
        tryResolve();
      });

      socket.on("end", () => {
        if (bodyComplete) return;
        const allData = Buffer.concat(chunks);

        if (!headerParsed) {
          const info = parseHttpResponseHeader(allData.toString("utf8"));
          if (info) {
            headerParsed = true;
            responseInfo = info;
            bodyStartOffset = info.headerLength;

            if (info.statusCode >= 300 && info.statusCode < 400 && info.headers.location) {
              bodyComplete = true;
              const redirectUrl = new URL(info.headers.location, targetUrl).toString();
              resolve(doRequest(redirectUrl, redirectDepth + 1));
              return;
            }
          } else {
            reject(new DownloadError(`Incomplete HTTP response from ${targetUrl}`, false));
            return;
          }
        }

        // End of socket — whatever we have is the body
        bodyComplete = true;
        const bodyBuf = allData.slice(bodyStartOffset);
        bodyStream.end(bodyBuf);
        resolve({
          statusCode: responseInfo.statusCode,
          headers: responseInfo.headers,
          bodyStream,
        });
      });

      socket.on("error", (err) => {
        reject(new DownloadError(`Network error: ${err.message}`));
      });

      socket.setTimeout(REQUEST_TIMEOUT_MS, () => {
        socket.destroy();
        reject(new DownloadError(`Response timed out: ${targetUrl}`));
      });
    });

    return resultPromise;
  }

  return doRequest(url, 0);
}

// ── HTTP download (binary to file) ────────────────────────────────

async function httpGet(url, destPath) {
  const { statusCode, bodyStream } = await rawHttpGet(url);
  if (statusCode !== 200) {
    bodyStream.resume();
    throw new DownloadError(`HTTP ${statusCode}: ${url}`, statusCode < 500);
  }

  return new Promise((resolve, reject) => {
    const file = createWriteStream(destPath);
    bodyStream.pipe(file);
    file.on("finish", () => {
      file.close();
      resolve();
    });
    file.on("error", (err) => {
      unlink(destPath).catch(() => {});
      reject(new DownloadError(`Write error: ${err.message}`));
    });
    bodyStream.on("error", (err) => {
      unlink(destPath).catch(() => {});
      reject(new DownloadError(`Network error: ${err.message}`));
    });
  });
}

// ── HTTP download (text to string) ────────────────────────────────

async function httpGetText(url) {
  const { statusCode, bodyStream } = await rawHttpGet(url);
  if (statusCode !== 200) {
    bodyStream.resume();
    throw new DownloadError(`HTTP ${statusCode}: ${url}`, statusCode < 500);
  }

  const chunks = [];
  return new Promise((resolve, reject) => {
    bodyStream.setEncoding("utf8");
    bodyStream.on("data", (chunk) => chunks.push(chunk));
    bodyStream.on("end", () => resolve(chunks.join("")));
    bodyStream.on("error", (err) => reject(new DownloadError(`Network error: ${err.message}`)));
  });
}

// ── Checksum ──────────────────────────────────────────────────────

async function sha256File(filePath) {
  const content = await readFile(filePath);
  return crypto.createHash("sha256").update(content).digest("hex");
}

async function downloadChecksums(version, repo) {
  const url = releaseAssetUrl(`tidev-sha256.txt`, version, repo);
  const text = await httpGetText(url);
  const map = new Map();
  for (const line of text.split(/\r?\n/)) {
    const m = line.trim().match(/^([a-f0-9]{64})\s+(.+)$/i);
    if (m) map.set(m[2], m[1].toLowerCase());
  }
  return map;
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
