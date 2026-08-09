#!/usr/bin/env bun
import { resolve } from "node:path";
import { existsSync, statSync } from "node:fs";
import { connect } from "node:net";
import { spawn } from "node:child_process";

import { resolveVaultRootFromTypFile } from "../src/shared/vault-root.ts";

const HOME = process.env.HOME ?? process.env.USERPROFILE ?? "";
const SOCKET_PATH = process.platform === "win32"
	? "\\\\.\\pipe\\typsmthng-cli"
	: HOME ? `${HOME}/.typsmthng/cli.sock` : "";
const arg = process.argv[2];

if (!arg) {
	console.log("Usage: typsmthng <path>");
	console.log("  typsmthng .          Open current directory as vault");
	console.log("  typsmthng folder/    Open folder as vault");
	console.log("  typsmthng file.typ   Open file (project root becomes vault)");
	process.exit(0);
}

const target = resolve(arg);

if (!existsSync(target)) {
	console.error(`typsmthng: path does not exist: ${target}`);
	process.exit(1);
}

const stat = statSync(target);
let vaultPath: string;
let selectFile: string | null = null;

if (stat.isFile()) {
	if (!target.endsWith(".typ")) {
		console.error(`typsmthng: not a .typ file: ${target}`);
		process.exit(1);
	}
	const resolved = resolveVaultRootFromTypFile(target);
	vaultPath = resolved.vaultPath;
	selectFile = resolved.selectFile;
} else if (stat.isDirectory()) {
	vaultPath = target;
} else {
	console.error(`typsmthng: not a file or directory: ${target}`);
	process.exit(1);
}

const message = JSON.stringify({ action: "open", path: vaultPath, selectFile });

function findWindowsAppBinary(): string | null {
	const localAppData = process.env.LOCALAPPDATA ?? "";
	const known = [
		localAppData ? resolve(localAppData, "typsmthng", "bin", "launcher.exe") : "",
		localAppData ? resolve(localAppData, "typsmthng", "typsmthng.exe") : "",
	];
	for (const candidate of known) {
		if (candidate && existsSync(candidate)) return candidate;
	}

	// Older installs may expose typsmthng.exe on PATH
	const pathEnv = process.env.PATH ?? process.env.Path ?? "";
	for (const dir of pathEnv.split(";")) {
		if (!dir) continue;
		const candidate = resolve(dir, "typsmthng.exe");
		if (existsSync(candidate)) return candidate;
	}

	return null;
}

// Try connecting to running instance via IPC socket
if (!SOCKET_PATH) {
	console.error("typsmthng: could not determine home directory");
	process.exit(1);
}

const client = connect(SOCKET_PATH);

client.on("connect", () => {
	client.write(message);
	client.end();
});

client.on("end", () => {
	process.exit(0);
});

client.on("error", () => {
	// No running instance — launch the app with the path as argument
	const platform = process.platform;

	if (platform === "darwin") {
		// macOS: use open command with spawn for safe argument handling
		const openArgs = ["-a", "typsmthng", "--args", vaultPath];
		if (selectFile) {
			openArgs.push("--select", selectFile);
		}
		const child = spawn("open", openArgs, { stdio: "inherit" });
		child.on("error", () => {
			console.error("typsmthng: failed to launch app. Is it installed?");
			process.exit(1);
		});
		child.on("close", (code) => {
			process.exit(code ?? 0);
		});
		return;
	} else if (platform === "linux") {
		// Linux: try to find the binary
		const candidates = [
			`${process.env.HOME}/.local/bin/typsmthng`,
			"/usr/local/bin/typsmthng",
			"/usr/bin/typsmthng",
		];

		let binary: string | null = null;
		for (const candidate of candidates) {
			if (existsSync(candidate)) {
				binary = candidate;
				break;
			}
		}

		if (!binary) {
			console.error("typsmthng: binary not found. Is it installed?");
			process.exit(1);
		}

		const args = [vaultPath];
		if (selectFile) {
			args.push("--select", selectFile);
		}

		spawn(binary, args, { detached: true, stdio: "ignore" }).unref();
	} else if (platform === "win32") {
		const args = [vaultPath];
		if (selectFile) {
			args.push("--select", selectFile);
		}

		const binary = findWindowsAppBinary();
		if (!binary) {
			console.error(
				"typsmthng: app not found. Expected launcher at %LOCALAPPDATA%\\typsmthng\\bin\\launcher.exe",
			);
			process.exit(1);
		}

		spawn(binary, args, { detached: true, stdio: "ignore" }).unref();
	} else {
		console.error(`typsmthng: unsupported platform: ${platform}`);
		process.exit(1);
	}

	process.exit(0);
});
