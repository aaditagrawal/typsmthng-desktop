#!/usr/bin/env bun
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const version = process.argv[2];
if (!version) {
  console.error("Usage: bun run scripts/update-version.ts <version>");
  console.error("Example: bun run scripts/update-version.ts 1.2.3");
  process.exit(1);
}

if (!/^\d+\.\d+\.\d+(-[\w.]+)?$/.test(version)) {
  console.error(`Invalid semver: ${version}`);
  process.exit(1);
}

const rootDir = join(import.meta.dir, "..");

// Update package.json
const pkgPath = join(rootDir, "package.json");
const pkg = JSON.parse(readFileSync(pkgPath, "utf-8"));
const oldVersion = pkg.version;
pkg.version = version;
writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
console.log(`package.json: ${oldVersion} → ${version}`);

// Update electrobun.config.ts
const configPath = join(rootDir, "electrobun.config.ts");
let config = readFileSync(configPath, "utf-8");
config = config.replace(
  /version:\s*"[^"]*"/,
  `version: "${version}"`,
);
writeFileSync(configPath, config);
console.log(`electrobun.config.ts: updated to ${version}`);
