/**
 * Pre-publish verification: ensure all release assets exist on GitHub.
 *
 * Run via `npm run prepublishOnly` (automatic before npm publish)
 * or manually: `node scripts/verify-release-assets.js`
 */

const https = require("https");
const http = require("http");
const { URL } = require("url");
const { releaseAssetUrl } = require("./artifacts");
const pkg = require("../package.json");

const ASSETS = [
  "tidev-sha256.txt",
  "tidev-linux-x64",
  "tidev-linux-arm64",
  "tidev-macos-x64",
  "tidev-macos-arm64",
  "tidev-windows-x64.exe",
];

async function head(url) {
  return new Promise((resolve, reject) => {
    const parsed = new URL(url);
    const client = parsed.protocol === "https:" ? https : http;
    const req = client.request(url, { method: "HEAD" }, (res) => {
      res.resume();
      resolve(res.statusCode === 200);
    });
    req.on("error", reject);
    req.end();
  });
}

async function main() {
  const version = pkg.tidevBinaryVersion || pkg.version;
  const repo = process.env.TIDEV_GITHUB_REPO || "Eslzzyl/tidev";
  let allOk = true;

  console.log(`Verifying release assets for tidev v${version} (repo: ${repo})...`);
  for (const asset of ASSETS) {
    const url = releaseAssetUrl(asset, version, repo);
    const ok = await head(url);
    console.log(`  ${ok ? "✓" : "✗"} ${asset}`);
    if (!ok) allOk = false;
  }

  if (!allOk) {
    console.error("\nMissing assets! Make sure the GitHub Release exists with all required files.");
    process.exit(1);
  }
  console.log("\nAll assets verified.");
}

main().catch((err) => {
  console.error("Verification failed:", err.message);
  process.exit(1);
});
