import { copy } from "../i18n";
type Labels = typeof copy.en | typeof copy["zh-CN"];
export function tunnelError(cause: unknown, labels: Labels): string {
  const code = typeof cause === "object" && cause !== null && "code" in cause ? cause.code : cause;
  switch (code) {
    case "bind_address_in_use": return labels.tunnelBindBusy;
    case "remote_forward_rejected": return labels.tunnelRemoteRejected;
    case "destination_unreachable": return labels.tunnelDestinationFailed;
    case "connection_lost": case "ssh_connection": return labels.tunnelConnectionLost;
    case "ssh_channel_rejected": return labels.tunnelChannelRejected;
    case "ssh_protocol": return labels.tunnelProtocolFailed;
    case "client_limit": return labels.tunnelClientLimit;
    case "invalid_config": case "invalid_input": return labels.tunnelInvalid;
    case "canceled": return labels.tunnelCanceled;
    case "vault_conflict": case "not_found": return labels.tunnelConflict;
    case "vault_locked": return labels.tunnelLocked;
    default: return labels.tunnelStorageFailed;
  }
}
