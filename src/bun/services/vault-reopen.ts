import type { AppMetadata } from "../../shared/rpc";

/** Whether bootstrap should restore the last vault from persisted app state. */
export function shouldReopenVault(
  metadata: Pick<AppMetadata, "reopenLastVaultPath">,
  pathExists: boolean,
): boolean {
  const reopenPath = metadata.reopenLastVaultPath;
  return Boolean(reopenPath) && pathExists;
}
