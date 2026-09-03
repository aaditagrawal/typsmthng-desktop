import { Electroview } from "electrobun/view";

import type {
	AppMetadata,
	DesktopRPC,
	ExternalVaultEvent,
	VaultRecord,
} from "../../shared/rpc";
import type {
	PresentationCommand,
	PresentationInput,
	PresentationSnapshot,
} from "../../shared/presentation";
import type { UpdateState } from "../../shared/update-types";

type ExternalVaultEventsPayload = {
	rootPath: string;
	events: ExternalVaultEvent[];
};

type Unsubscribe = () => void;

const externalVaultListeners = new Set<
	(payload: ExternalVaultEventsPayload) => void
>();
const metadataListeners = new Set<(metadata: AppMetadata) => void>();
const activeVaultOpenedListeners = new Set<(vault: VaultRecord) => void>();
const activeVaultClosedListeners = new Set<() => void>();
const updateStateListeners = new Set<(state: UpdateState) => void>();
const presentationSnapshotListeners = new Set<(snapshot: PresentationSnapshot) => void>();
const presentationInputListeners = new Set<(input: PresentationInput) => void>();
const presentationAudienceClosedListeners = new Set<() => void>();
const presentationCommandListeners = new Set<(command: PresentationCommand) => void>();

function subscribe<T>(
	listeners: Set<(value: T) => void>,
	listener: (value: T) => void,
): Unsubscribe {
	listeners.add(listener);
	return () => {
		listeners.delete(listener);
	};
}

const desktopRpc = Electroview.defineRPC<DesktopRPC>({
	maxRequestTime: 5 * 60 * 1000,
	handlers: {
		messages: {
			updateStateChanged(state) {
				for (const listener of updateStateListeners) {
					listener(state);
				}
			},
			externalVaultEvents(payload) {
				for (const listener of externalVaultListeners) {
					listener(payload);
				}
			},
			metadataUpdated(metadata) {
				for (const listener of metadataListeners) {
					listener(metadata);
				}
			},
			activeVaultOpened(vault) {
				for (const listener of activeVaultOpenedListeners) {
					listener(vault);
				}
			},
			activeVaultClosed() {
				for (const listener of activeVaultClosedListeners) {
					listener();
				}
			},
			presentationSnapshot(snapshot) {
				for (const listener of presentationSnapshotListeners) {
					listener(snapshot);
				}
			},
			presentationInput(input) {
				for (const listener of presentationInputListeners) {
					listener(input);
				}
			},
			presentationAudienceClosed() {
				for (const listener of presentationAudienceClosedListeners) {
					listener();
				}
			},
			presentationCommand(command) {
				for (const listener of presentationCommandListeners) {
					listener(command);
				}
			},
		},
	},
});

const electroview = new Electroview({ rpc: desktopRpc });

export function onExternalVaultEvents(
	listener: (payload: ExternalVaultEventsPayload) => void,
): Unsubscribe {
	return subscribe(externalVaultListeners, listener);
}

export function onMetadataUpdated(
	listener: (metadata: AppMetadata) => void,
): Unsubscribe {
	return subscribe(metadataListeners, listener);
}

export function onActiveVaultOpened(
	listener: (vault: VaultRecord) => void,
): Unsubscribe {
	return subscribe(activeVaultOpenedListeners, listener);
}

export function onActiveVaultClosed(listener: () => void): Unsubscribe {
	activeVaultClosedListeners.add(listener);
	return () => {
		activeVaultClosedListeners.delete(listener);
	};
}

export function onUpdateStateChanged(
	listener: (state: UpdateState) => void,
): Unsubscribe {
	return subscribe(updateStateListeners, listener);
}

export function onPresentationSnapshot(
	listener: (snapshot: PresentationSnapshot) => void,
): Unsubscribe {
	return subscribe(presentationSnapshotListeners, listener);
}

export function onPresentationInput(
	listener: (input: PresentationInput) => void,
): Unsubscribe {
	return subscribe(presentationInputListeners, listener);
}

export function onPresentationAudienceClosed(listener: () => void): Unsubscribe {
	presentationAudienceClosedListeners.add(listener);
	return () => {
		presentationAudienceClosedListeners.delete(listener);
	};
}

export function onPresentationCommand(
	listener: (command: PresentationCommand) => void,
): Unsubscribe {
	return subscribe(presentationCommandListeners, listener);
}

export { desktopRpc, electroview };
