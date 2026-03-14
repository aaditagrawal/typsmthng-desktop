export type UpdateStatus =
	| "disabled"
	| "idle"
	| "checking"
	| "up-to-date"
	| "available"
	| "downloading"
	| "ready"
	| "error";

export interface UpdateState {
	status: UpdateStatus;
	currentVersion: string;
	availableVersion: string | null;
	error: string | null;
}
