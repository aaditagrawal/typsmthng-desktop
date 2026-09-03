#!/usr/bin/env bun
import { writeVersionJsonFile } from "../src/bun/services/version-info.ts";

const dest = process.argv[2];
if (!dest) {
	console.error("Usage: bun scripts/write-version-json.ts <dest-version.json>");
	process.exit(1);
}

const channel = process.env.ELECTROBUN_ENV || "stable";
const info = writeVersionJsonFile(dest, { channel });
console.log(`Wrote ${dest}`);
console.log(JSON.stringify(info, null, 2));
