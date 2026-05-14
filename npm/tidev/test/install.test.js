const { describe, it } = require("node:test");
const assert = require("node:assert");
const { detectBinaryName, releaseAssetUrl, releaseBinaryDirectory } = require("../scripts/artifacts");

describe("artifacts", () => {
  it("should detect binary name for current platform", () => {
    const result = detectBinaryName();
    assert.ok(["linux", "darwin", "win32"].includes(result.platform));
    assert.ok(["x64", "arm64"].includes(result.arch));
    assert.ok(typeof result.name === "string");
    assert.ok(result.name.length > 0);
  });

  it("should generate a valid release asset URL", () => {
    const url = releaseAssetUrl("tidev-linux-x64", "0.3.0", "Eslzzyl/tidev");
    assert.ok(url.includes("github.com"));
    assert.ok(url.includes("releases"));
    assert.ok(url.includes("v0.3.0"));
    assert.ok(url.includes("tidev-linux-x64"));
  });

  it("should return a binary directory path", () => {
    const dir = releaseBinaryDirectory();
    assert.ok(dir.endsWith("downloads"));
  });
});
