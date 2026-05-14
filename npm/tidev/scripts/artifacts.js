const path = require("path");
const os = require("os");

const TARGET_MATRIX = {
  linux: {
    x64: "tidev-linux-x64",
    arm64: "tidev-linux-arm64",
  },
  darwin: {
    x64: "tidev-macos-x64",
    arm64: "tidev-macos-arm64",
  },
  win32: {
    x64: "tidev-windows-x64.exe",
  },
};

function detectBinaryName() {
  const platform = os.platform();
  const arch = os.arch();
  const entry = TARGET_MATRIX[platform];
  if (!entry) {
    throw new Error(
      `Unsupported platform: ${platform}. Supported: ${Object.keys(TARGET_MATRIX).join(", ")}`,
    );
  }
  const name = entry[arch];
  if (!name) {
    throw new Error(
      `Unsupported architecture: ${arch} on ${platform}. Supported: ${Object.keys(entry).join(", ")}`,
    );
  }
  return { platform, arch, name };
}

function releaseBaseUrl(version, repo = "Eslzzyl/tidev") {
  const override = process.env.TIDEV_RELEASE_BASE_URL;
  if (override) {
    const trimmed = String(override).trim();
    return trimmed.endsWith("/") ? trimmed : `${trimmed}/`;
  }
  return `https://github.com/${repo}/releases/download/v${version}/`;
}

function releaseAssetUrl(name, version, repo) {
  return new URL(name, releaseBaseUrl(version, repo)).toString();
}

function releaseBinaryDirectory() {
  return path.join(__dirname, "..", "bin", "downloads");
}

module.exports = {
  detectBinaryName,
  releaseBaseUrl,
  releaseAssetUrl,
  releaseBinaryDirectory,
};
