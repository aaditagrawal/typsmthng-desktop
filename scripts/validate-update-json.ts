#!/usr/bin/env bun
const dest = process.argv[2];
if (!dest) {
	console.error("Usage: bun scripts/validate-update-json.ts <update.json>");
	process.exit(1);
}

const parsed = JSON.parse(await Bun.file(dest).text()) as { version?: unknown; hash?: unknown };
if (typeof parsed.hash !== "string" || !parsed.hash) {
	console.error("update.json missing hash:", dest);
	process.exit(1);
}
if (typeof parsed.version !== "string" || !parsed.version) {
	console.error("update.json missing version:", dest);
	process.exit(1);
}
console.log(`update.json version=${parsed.version} hash=${parsed.hash}`);
