import { describe, expect, it } from "vitest";
import { shouldReopenVault } from "./vault-reopen";

describe("shouldReopenVault", () => {
  it("reopens when path is set and exists", () => {
    expect(
      shouldReopenVault({ reopenLastVaultPath: "/docs/thesis" }, true),
    ).toBe(true);
  });

  it("does not reopen when path is null", () => {
    expect(shouldReopenVault({ reopenLastVaultPath: null }, true)).toBe(false);
  });

  it("does not reopen when path is empty", () => {
    expect(shouldReopenVault({ reopenLastVaultPath: "" }, true)).toBe(false);
  });

  it("does not reopen when directory is missing", () => {
    expect(
      shouldReopenVault({ reopenLastVaultPath: "/missing/vault" }, false),
    ).toBe(false);
  });
});
