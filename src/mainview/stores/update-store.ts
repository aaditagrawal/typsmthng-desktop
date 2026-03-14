import { create } from "zustand";
import { desktopRpc, onUpdateStateChanged } from "@/lib/desktop-rpc";
import type { UpdateState, UpdateStatus } from "../../shared/update-types";

interface UpdateStore extends UpdateState {
	init: () => () => void;
	checkForUpdate: () => Promise<void>;
	downloadUpdate: () => Promise<void>;
	applyUpdate: () => Promise<void>;
}

export const useUpdateStore = create<UpdateStore>((set) => ({
	status: "idle" as UpdateStatus,
	currentVersion: "0.1.0",
	availableVersion: null,
	error: null,

	init: () => {
		return onUpdateStateChanged((state) => {
			set(state);
		});
	},

	checkForUpdate: async () => {
		const state = await desktopRpc.request.checkForUpdate();
		set(state);
	},

	downloadUpdate: async () => {
		const state = await desktopRpc.request.downloadUpdate();
		set(state);
	},

	applyUpdate: async () => {
		await desktopRpc.request.applyUpdate();
	},
}));
