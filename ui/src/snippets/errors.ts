type Labels = {
  snippetConflict: string;
  snippetVaultLocked: string;
  snippetStorageFailed: string;
  snippetGroupInUse: string;
  snippetInvalid: string;
};

export function snippetMutationError(cause: unknown, labels: Labels): string {
  const code = typeof cause === "object" && cause !== null && "code" in cause ? cause.code : undefined;
  switch (code) {
    case "vault_conflict":
    case "not_found": return labels.snippetConflict;
    case "vault_locked": return labels.snippetVaultLocked;
    case "group_in_use": return labels.snippetGroupInUse;
    case "invalid_input": return labels.snippetInvalid;
    default: return labels.snippetStorageFailed;
  }
}
