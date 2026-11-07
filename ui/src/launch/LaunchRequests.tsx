import { useEffect, useRef, useState } from "react";
import { invokeTauri, isTauriAvailable, listenTauriEvent } from "../terminal/bridge";
import type { LaunchRequestView } from "../terminal/contracts";

function isLaunchRequest(value: unknown): value is LaunchRequestView {
  return typeof value === "object" && value !== null
    && "requestId" in value && typeof value.requestId === "string"
    && "kind" in value && ["focus", "local", "host", "ssh"].includes(String(value.kind))
    && "display" in value && typeof value.display === "string";
}

export function LaunchRequests({ onRequest }: { onRequest: (request: LaunchRequestView) => Promise<void> }): null {
  const [requests, setRequests] = useState<LaunchRequestView[]>([]);
  const seen = useRef(new Set<string>());
  const currentHandler = useRef(onRequest);
  const processing = useRef(false);
  const mounted = useRef(false);
  const appendRequest = useRef<(value: unknown) => void>(() => undefined);
  currentHandler.current = onRequest;

  useEffect(() => {
    if (!isTauriAvailable()) return;
    let active = true;
    mounted.current = true;
    const append = (value: unknown): void => {
      if (!active || !isLaunchRequest(value) || seen.current.has(value.requestId)) return;
      setRequests((current) => current.some(({ requestId }) => requestId === value.requestId)
        ? current : [...current, value].slice(0, 8));
    };
    appendRequest.current = append;
    let unlisten: (() => void) | undefined;
    void listenTauriEvent<unknown>("launch-request", append).then((stop) => {
      if (active) unlisten = stop;
      else stop();
      // Subscribe before reading the queue so startup cannot lose a forwarded request.
      return invokeTauri("list_launch_requests");
    }).then((pending) => {
      if (active && Array.isArray(pending)) pending.forEach(append);
    }).catch(() => undefined);
    return () => { active = false; mounted.current = false; unlisten?.(); };
  }, []);

  useEffect(() => {
    const request = requests[0];
    if (!request || processing.current) return;
    processing.current = true;
    void currentHandler.current(request).catch(() => undefined).finally(() => {
      seen.current.add(request.requestId);
      if (seen.current.size > 64) seen.current.delete(seen.current.values().next().value ?? "");
      processing.current = false;
      if (mounted.current) {
        setRequests((current) => current.filter(({ requestId }) => requestId !== request.requestId));
        void invokeTauri("list_launch_requests").then((pending) => {
          if (Array.isArray(pending)) pending.forEach((request) => appendRequest.current(request));
        }).catch(() => undefined);
      }
    });
  }, [requests]);
  return null;
}
