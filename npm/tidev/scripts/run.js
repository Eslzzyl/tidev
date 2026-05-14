const { spawnSync } = require("child_process");
const { getBinaryPath } = require("./install");

async function runTidev() {
  const binaryPath = await getBinaryPath();
  const result = spawnSync(binaryPath, process.argv.slice(2), {
    stdio: "inherit",
  });
  if (result.error) {
    throw result.error;
  }
  process.exit(result.status ?? 1);
}

module.exports = { runTidev };
