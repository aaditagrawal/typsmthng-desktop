import { existsSync, statSync } from "node:fs";
import { basename, dirname, join, relative, resolve, sep } from "node:path";

const VAULT_ROOT_MARKERS = [
	"typst.toml",
	"main.typ",
	".typsmthng",
	".git",
] as const;

function isVaultRootMarker(dir: string, marker: (typeof VAULT_ROOT_MARKERS)[number]): boolean {
	const candidate = join(dir, marker);
	if (!existsSync(candidate)) return false;

	if (marker === ".git") {
		try {
			return statSync(candidate).isDirectory();
		} catch {
			return false;
		}
	}

	// typst.toml / main.typ: any existing entry counts
	// .typsmthng: file or directory
	return true;
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
