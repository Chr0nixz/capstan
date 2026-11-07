import { useId, useRef, useState } from "react";
import type { copy } from "../i18n";
import { formatBytes } from "../ui/format";
import type { ServerStatusSnapshot } from "./contracts";
import { retryServerStatus } from "./useServerStatus";

type Labels = typeof copy[keyof typeof copy]["serverStatus"];

export function ServerStatusBar({ host, snapshot, ended, labels }: { host: string; snapshot?: ServerStatusSnapshot; ended?: boolean; labels: Labels }) {
  const [expanded, setExpanded] = useState(false);
  const detailsId = useId();
  const metricsId = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const state = snapshot?.state ?? (ended ? "unavailable" : "probing");
  const missing = labels.missing;
  const memory = snapshot?.memory, disk = snapshot?.disk, rate = snapshot?.network;
  const statusText = snapshot?.errorCode ? labels.errors[snapshot.errorCode] : labels.states[state];
  return <section className="server-status" aria-label={labels.title} onKeyDown={(event) => {
    if (event.key === "Escape" && expanded) {
      event.preventDefault(); event.stopPropagation(); setExpanded(false); trigger.current?.focus();
    }
  }}>
    <button type="button" className="server-status-summary" ref={trigger} aria-controls={detailsId} aria-expanded={expanded}
      aria-label={labels.details(host)} aria-describedby={metricsId} onClick={() => setExpanded(!expanded)}>
      <span className="server-status-host" title={host}><bdi>{host}</bdi></span>
      <span id={metricsId} className="server-status-metrics">
      <span className="server-status-metric">{labels.cpu} {snapshot?.cpuPercent === undefined ? missing : `${snapshot.cpuPercent.toFixed(0)}%`}</span>
      <span className="server-status-metric">{labels.memory} {memory ? `${formatBytes(String(BigInt(memory.total) - BigInt(memory.available)))} / ${formatBytes(memory.total)}` : missing}</span>
      <span className="server-status-metric">{labels.disk} {disk ? `${formatBytes(disk.used)} / ${formatBytes(disk.total)}` : missing}</span>
      <span className="server-status-metric">{labels.receive} {rate ? labels.perSecond(formatBytes(rate.rxPerSec)) : missing}</span>
      <span className="server-status-metric">{labels.send} {rate ? labels.perSecond(formatBytes(rate.txPerSec)) : missing}</span>
      {state !== "running" && <span className="server-status-state">{labels.states[state]}</span>}
      </span>
      <span className="server-status-chevron" aria-hidden="true">{expanded ? "⌄" : "⌃"}</span>
    </button>
    <div id={detailsId} className="server-status-details" hidden={!expanded} tabIndex={0} role="region" aria-label={labels.details(host)}>
      <header>
        <strong><bdi>{host}</bdi></strong>
        <span>{statusText}</span>
        {snapshot?.state === "unavailable" && <button type="button" onClick={() => void retryServerStatus(snapshot).catch(() => undefined)}>{labels.retry}</button>}
      </header>
      {snapshot?.details && <>
        <p>{labels.load} {snapshot.load?.map((value) => value.toFixed(2)).join(" / ") ?? missing}</p>
        <h3>{labels.mounts}</h3>
        <ul>{snapshot.details.disks.map((disk, index) => <li key={`${disk.mount}:${index}`}>
          <bdi>{disk.mount}</bdi><span>{formatBytes(disk.used)} / {formatBytes(disk.total)}</span>
        </li>)}</ul>
        <h3>{labels.interfaces}</h3>
        <ul>{snapshot.details.interfaces.map(({ name, rate }) => <li key={name}>
          <bdi>{name}</bdi><span>{labels.receive} {rate ? labels.perSecond(formatBytes(rate.rxPerSec)) : missing} · {labels.send} {rate ? labels.perSecond(formatBytes(rate.txPerSec)) : missing}</span>
        </li>)}</ul>
      </>}
    </div>
  </section>;
}
