import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { resolveVaultRootFromTypFile } from "./vault-root";

const tempDirs: string[] = [];

function makeTempRoot(): string {
	const dir = mkdtempSync(join(tmpdir(), "vault-root-"));
	tempDirs.push(dir);
	return dir;
}

afterEach(() => {
	while (tempDirs.length > 0) {
		const dir = tempDirs.pop();
		if (!dir) continue;
		rmSync(dir, { recursive: true, force: true });
	}
});

describe("resolveVaultRootFromTypFile", () => {
	it("uses the directory next to main.typ", () => {
		const root = makeTempRoot();
		writeFileSync(join(root, "main.typ"), "= Doc\n");
		writeFileSync(join(root, "notes.typ"), "= Notes\n");

		expect(resolveVaultRootFromTypFile(join(root, "notes.typ"))).toEqual({
			vaultPath: root,
			selectFile: "notes.typ",
		});
	});

	it("walks up from chapters/ when main.typ is at the project root", () => {
		const root = makeTempRoot();
		mkdirSync(join(root, "chapters"));
		writeFileSync(join(root, "main.typ"), '= Doc\n#include "chapters/intro.typ"\n');
		writeFileSync(join(root, "chapters", "intro.typ"), "= Intro\n");

		expect(resolveVaultRootFromTypFile(join(root, "chapters", "intro.typ"))).toEqual({
			vaultPath: root,
			selectFile: "chapters/intro.typ",
		});
	});

	it("uses a root that only has .git", () => {
		const root = makeTempRoot();
		mkdirSync(join(root, ".git"));
		mkdirSync(join(root, "src"));
		writeFileSync(join(root, "src", "paper.typ"), "= Paper\n");

		expect(resolveVaultRootFromTypFile(join(root, "src", "paper.typ"))).toEqual({
			vaultPath: root,
			selectFile: "src/paper.typ",
		});
	});

	it("falls back to the immediate parent when no markers exist", () => {
		const root = makeTempRoot();
		mkdirSync(join(root, "lonely"));
		writeFileSync(join(root, "lonely", "solo.typ"), "= Solo\n");

		expect(resolveVaultRootFromTypFile(join(root, "lonely", "solo.typ"))).toEqual({
			vaultPath: join(root, "lonely"),
			selectFile: "solo.typ",
		});
	});
});
