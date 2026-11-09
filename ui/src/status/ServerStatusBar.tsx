import { useId, useRef, useState, type ReactNode } from "react";
import type { copy } from "../i18n";
import { formatBytes } from "../ui/format";
import { ArrowDownIcon, ArrowUpIcon, ChevronDownIcon } from "../ui/icons";
import type { TabStatus } from "../workspace";
import type { ServerStatusSnapshot } from "./contracts";
import { retryServerStatus } from "./useServerStatus";

type Labels = typeof copy[keyof typeof copy]["serverStatus"];

/** Usage at or above this share lights a metric; below it the bar stays quiet. */
export const highUsage = 0.9;

/** used / total as a 0–1 share, or null when the total is unknown or zero. */
export function usageShare(used: string, total: string): number | null {
  const whole = BigInt(total);
  return whole > 0n ? Number((BigInt(used) * 1000n) / whole) / 1000 : null;
}

function Metric({ label, high, highLabel, children }: { label: ReactNode; high: boolean; highLabel: string; children: ReactNode }) {
  return <span className="server-status-metric" data-level={high ? "high" : undefined}>
    <span className="server-status-label">{label}</span> {children}{high && <span className="sr-only"> ({highLabel})</span>}
  </span>;
}

type Props = {
  host: string; snapshot?: ServerStatusSnapshot; ended?: boolean; labels: Labels;
  /** The session light of the focused pane, identical to its tab's. */
  signal?: TabStatus;
  /** Chip text when the host is production. */
  production?: string;
};

export function ServerStatusBar({ host, snapshot, ended, labels, signal, production }: Props) {
  const [expanded, setExpanded] = useState(false);
  const detailsId = useId();
  const metricsId = useId();
  const trigger = useRef<HTMLButtonElement>(null);
  const state = snapshot?.state ?? (ended ? "unavailable" : "probing");
  const missing = labels.missing;
  const memory = snapshot?.memory, disk = snapshot?.disk, rate = snapshot?.network;
  const statusText = snapshot?.errorCode ? labels.errors[snapshot.errorCode] : labels.states[state];
  const memoryUsed = memory ? String(BigInt(memory.total) - BigInt(memory.available)) : null;
  const memoryShare = memory && memoryUsed ? usageShare(memoryUsed, memory.total) : null;
  const diskShare = disk ? usageShare(disk.used, disk.total) : null;
  // Until the first sample there is nothing to read: one state line instead of a row of dashes.
  const sampled = snapshot?.cpuPercent !== undefined || !!memory || !!disk || !!rate;
  const light = signal === "starting" ? "connecting" : signal;
  return <section className="server-status" aria-label={labels.title} data-expanded={expanded || undefined} onKeyDown={(event) => {
    if (event.key === "Escape" && expanded) {
      event.preventDefault(); event.stopPropagation(); setExpanded(false); trigger.current?.focus();
    }
  }}>
    <button type="button" className="server-status-summary" ref={trigger} aria-controls={detailsId} aria-expanded={expanded}
      aria-label={labels.details(host)} aria-describedby={metricsId} onClick={() => setExpanded(!expanded)}>
      {light && <span className="led" data-state={light} aria-hidden="true" />}
      <span className="server-status-host" title={host}><bdi>{host}</bdi></span>
      {production && <span className="prod-chip">{production}</span>}
      <span id={metricsId} className="server-status-metrics">
      {sampled ? <>
        <Metric label={labels.cpu} high={(snapshot?.cpuPercent ?? 0) >= highUsage * 100} highLabel={labels.high}>{snapshot?.cpuPercent === undefined ? missing : `${snapshot.cpuPercent.toFixed(0)}%`}</Metric>
        <Metric label={labels.memory} high={(memoryShare ?? 0) >= highUsage} highLabel={labels.high}>{memory && memoryUsed ? `${formatBytes(memoryUsed)} / ${formatBytes(memory.total)}` : missing}</Metric>
        <Metric label={labels.disk} high={(diskShare ?? 0) >= highUsage} highLabel={labels.high}>{disk ? `${formatBytes(disk.used)} / ${formatBytes(disk.total)}` : missing}</Metric>
        <Metric label={<><ArrowDownIcon size={11} /><span className="sr-only">{labels.receive}</span></>} high={false} highLabel={labels.high}>{rate ? labels.perSecond(formatBytes(rate.rxPerSec)) : missing}</Metric>
        <Metric label={<><ArrowUpIcon size={11} /><span className="sr-only">{labels.send}</span></>} high={false} highLabel={labels.high}>{rate ? labels.perSecond(formatBytes(rate.txPerSec)) : missing}</Metric>
        {state !== "running" && <span className="server-status-state">{labels.states[state]}</span>}
      </> : <span className="server-status-state">{labels.states[state]}</span>}
      </span>
      <ChevronDownIcon size={12} className="server-status-chevron" />
    </button>
    <div id={detailsId} className="server-status-details" hidden={!expanded} tabIndex={0} role="region" aria-label={labels.details(host)}>
      <header>
        <strong><bdi>{host}</bdi>{production && <span className="prod-chip">{production}</span>}</strong>
        <span>{statusText}</span>
        {snapshot?.state === "unavailable" && <button type="button" onClick={() => void retryServerStatus(snapshot).catch(() => undefined)}>{labels.retry}</button>}
      </header>
      {snapshot?.details && <>
        <p>{labels.load} {snapshot.load?.map((value) => value.toFixed(2)).join(" / ") ?? missing}</p>
        <h3>{labels.mounts}</h3>
        <ul>{snapshot.details.disks.map((disk, index) => <li key={`${disk.mount}:${index}`} data-level={(usageShare(disk.used, disk.total) ?? 0) >= highUsage ? "high" : undefined}>
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
