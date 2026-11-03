import { useEffect, useRef, useState, type ReactElement } from "react";
import { copy } from "../i18n";
import { invokeTauri } from "../terminal/bridge";
import { Titlebar } from "../window/Titlebar";
import { benchmarkCopy } from "./copy";
import { runBenchmarks } from "./runner";
import { percentile } from "./ledger";
import type { Report, Run } from "./contracts";

export function BenchmarkApp(): ReactElement {
  const locale = navigator.language.startsWith("zh") ? "zh-CN" : "en";
  const labels = benchmarkCopy[locale];
  const host = useRef<HTMLDivElement>(null);
  const active = useRef<AbortController | undefined>(undefined);
  const [enabled, setEnabled] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(false);
  const [progress, setProgress] = useState<{ kind: Run["kind"]; round: number }>();
  const [report, setReport] = useState<Report>();
  const run = (): void => {
    if (!host.current || active.current) return;
    const cancellation = new AbortController();
    active.current = cancellation;
    setBusy(true); setError(false);
    void runBenchmarks(host.current, cancellation.signal, (kind, round) => setProgress({ kind, round }))
      .then((result) => { setReport(result); setError(result.runs.some((scenario) => scenario.failure !== null)); })
      .catch(() => setError(true)).finally(() => { active.current = undefined; setBusy(false); });
  };
  useEffect(() => {
    let mounted = true;
    void invokeTauri("m0_benchmark_enabled").then((available) => {
      if (mounted) { setEnabled(available); if (available) run(); }
    }).catch(() => { if (mounted) setEnabled(false); });
    return () => { mounted = false; active.current?.abort(); };
  }, []);
  const response = report ? percentile(report.runs.flatMap((scenario) => scenario.input_response_ms), 0.95) : null;
  return <main className="workspace benchmark-workspace" aria-label={labels.title}>
    <Titlebar labels={copy[locale]} context={labels.title} />
    <div className="benchmark-toolbar">
      <span role="status">{error ? labels.failed : report ? labels.saved : progress ? `${labels[progress.kind]} ${labels.round(progress.round)}` : enabled ? labels.ready : labels.disabled}</span>
      <button type="button" disabled={!enabled || busy || !!report} onClick={run}>{labels.start}</button>
      <button type="button" disabled={!busy} onClick={() => active.current?.abort()}>{labels.cancel}</button>
    </div>
    <section className="terminal-shell benchmark-shell">
      <div className="benchmark-caption">{labels.description}{report && <span>{labels.done(report.runs.length)} · {labels.latency(response)}</span>}</div>
      <div ref={host} className="benchmark-host" />
    </section>
  </main>;
}
