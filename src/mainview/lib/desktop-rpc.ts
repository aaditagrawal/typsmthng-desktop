import { Electroview } from "electrobun/view";

import type {
	AppMetadata,
	DesktopRPC,
	ExternalVaultEvent,
} from "../../shared/rpc";

type ExternalVaultEventsPayload = {
	rootPath: string;
	events: ExternalVaultEvent[];
};

type Unsubscribe = () => void;

const externalVaultListeners = new Set<
	(payload: ExternalVaultEventsPayload) => void
>();
const metadataListeners = new Set<(metadata: AppMetadata) => void>();
const activeVaultClosedListeners = new Set<() => void>();

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
	handlers: {
		messages: {
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
			activeVaultClosed() {
				for (const listener of activeVaultClosedListeners) {
					listener();
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

export function onActiveVaultClosed(listener: () => void): Unsubscribe {
	activeVaultClosedListeners.add(listener);
	return () => {
		activeVaultClosedListeners.delete(listener);
	};
}

export { desktopRpc, electroview };
