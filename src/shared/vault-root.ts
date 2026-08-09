import { existsSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";

// Prefer Typst/project markers only. Do not treat ancestor `.git` as a vault root —
// that can resolve a lone .typ under a large git checkout (or $HOME) to the wrong folder.
const VAULT_ROOT_MARKERS = [
	"typst.toml",
	"main.typ",
	".typsmthng",
] as const;

function isVaultRootMarker(dir: string, marker: (typeof VAULT_ROOT_MARKERS)[number]): boolean {
	return existsSync(join(dir, marker));
}

function directoryHasVaultRootMarker(dir: string): boolean {
	return VAULT_ROOT_MARKERS.some((marker) => isVaultRootMarker(dir, marker));
}

function toPosixRelative(from: string, to: string): string {
	return relative(from, to).split(sep).join("/");
}

/** Walk up from a .typ file to find the project/vault root via known markers. */
export function resolveVaultRootFromTypFile(absoluteFilePath: string): {
	vaultPath: string;
	selectFile: string;
} {
	const filePath = resolve(absoluteFilePath);
	const immediateParent = dirname(filePath);

	let dir = immediateParent;
	for (;;) {
		if (directoryHasVaultRootMarker(dir)) {
			return {
				vaultPath: dir,
				selectFile: toPosixRelative(dir, filePath),
			};
		}

		const parent = dirname(dir);
		if (parent === dir) break;
		dir = parent;
	}

	return {
		vaultPath: immediateParent,
		selectFile: basename(filePath),
	};
}
