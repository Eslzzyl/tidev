#!/usr/bin/env node

const { runTidev } = require("../scripts/run");

runTidev().catch((error) => {
  console.error("Failed to start tidev:", error.message);
  process.exit(1);
});
