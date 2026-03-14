import { useEffect } from "react";
import { RefreshCw, Download, X, AlertCircle, CheckCircle } from "lucide-react";
import { useUpdateStore } from "@/stores/update-store";

export function UpdateBanner() {
	const { status, availableVersion, error, init, downloadUpdate, applyUpdate } =
		useUpdateStore();

	useEffect(() => {
		const unsub = init();
		return unsub;
	}, [init]);

	if (status === "idle" || status === "disabled" || status === "up-to-date") {
		return null;
	}

	if (status === "checking") {
		return (
			<Banner>
				<RefreshCw size={14} className="animate-spin" style={{ flexShrink: 0 }} />
				<span>Checking for updates...</span>
			</Banner>
		);
	}

	if (status === "available") {
		return (
			<Banner>
				<Download size={14} style={{ flexShrink: 0 }} />
				<span>
					Update {availableVersion ? `v${availableVersion} ` : ""}available
				</span>
				<BannerAction onClick={() => void downloadUpdate()}>
					Download
				</BannerAction>
			</Banner>
		);
	}

	if (status === "downloading") {
		return (
			<Banner>
				<RefreshCw size={14} className="animate-spin" style={{ flexShrink: 0 }} />
				<span>Downloading update...</span>
			</Banner>
		);
	}

	if (status === "ready") {
		return (
			<Banner>
				<CheckCircle size={14} style={{ flexShrink: 0 }} />
				<span>Update ready</span>
				<BannerAction onClick={() => void applyUpdate()}>
					Restart to apply
				</BannerAction>
			</Banner>
		);
	}

	if (status === "error") {
		return (
			<Banner>
				<AlertCircle size={14} style={{ flexShrink: 0 }} />
				<span>Update failed{error ? `: ${error}` : ""}</span>
				<DismissButton />
			</Banner>
		);
	}

	return null;
}

function Banner({ children }: { children: React.ReactNode }) {
	return (
		<div
			style={{
				display: "flex",
				alignItems: "center",
				gap: "8px",
				padding: "4px 12px",
				background: "var(--bg-elevated)",
				borderBottom: "1px solid var(--border-default)",
				fontFamily: "var(--font-mono)",
				fontSize: "11px",
				color: "var(--text-secondary)",
				letterSpacing: "0.02em",
			}}
		>
			{children}
		</div>
	);
}

function BannerAction({
	children,
	onClick,
}: {
	children: React.ReactNode;
	onClick: () => void;
}) {
	return (
		<button
			onClick={onClick}
			style={{
				background: "var(--accent)",
				color: "#fff",
				border: "none",
				borderRadius: "2px",
				padding: "2px 8px",
				fontSize: "11px",
				fontFamily: "var(--font-mono)",
				cursor: "pointer",
				letterSpacing: "0.02em",
			}}
		>
			{children}
		</button>
	);
}

function DismissButton() {
	const reset = () =>
		useUpdateStore.setState({ status: "idle", error: null });

	return (
		<button
			onClick={reset}
			style={{
				background: "none",
				border: "none",
				color: "var(--text-tertiary)",
				cursor: "pointer",
				padding: "2px",
				display: "inline-flex",
				alignItems: "center",
				marginLeft: "auto",
			}}
			title="Dismiss"
		>
			<X size={14} />
		</button>
	);
}
