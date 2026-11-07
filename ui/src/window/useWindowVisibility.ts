import { useEffect, useState } from "react";
import { isTauriAvailable, listenTauriEvent } from "../terminal/bridge";

export function useWindowVisibility(onShow: () => void, onCwdFallback: () => void): boolean {
  const [visible, setVisible] = useState(true);
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let active = true;
    const stops: (() => void)[] = [];
    const install = (operation: Promise<() => void>): void => {
      void operation.then((stop) => { if (active) stops.push(stop); else stop(); }).catch(() => undefined);
    };
    install(listenTauriEvent<boolean>("quake-visibility-changed", (next) => {
      if (!active || typeof next !== "boolean") return;
      setVisible(next);
      if (next) onShow();
    }));
    install(listenTauriEvent("launch-cwd-fallback", () => { if (active) onCwdFallback(); }));
    return () => { active = false; stops.forEach((stop) => stop()); };
  }, [onShow, onCwdFallback]);
  return visible;
}
