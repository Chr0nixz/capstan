import { createTauriChannel, invokeTauri } from "../terminal/bridge";
import { createTerminalController } from "../terminal/controller";
import { createTerminalSession } from "../terminal/session";
import type { TerminalTransport } from "../terminal/transport";
import type { TerminalSessionId, TerminalSnapshot } from "../terminal/contracts";
import { ProbeMarkers, ReceiverLedger } from "./ledger";
import type { Failure, Report, Run, Scenario } from "./contracts";
import { terminalPlan, type FixtureBackend } from "./plan";
import { BenchmarkError, until, withDeadline } from "./deadline";

function openTerminal(host: HTMLElement, scenario: Scenario, holdAck: boolean, responses: number[], backend: FixtureBackend) {
  const element = document.createElement("div");
  element.className = "benchmark-terminal";
  element.hidden = true;
  host.appendChild(element);
  const output = createTauriChannel<ArrayBuffer>();
  const status = createTauriChannel<TerminalSnapshot>();
  if (!output || !status) throw new BenchmarkError("backend");
  const ledger = new ReceiverLedger();
  let id: TerminalSessionId | undefined;
  let snapshot: TerminalSnapshot | undefined;
  let failure: Failure | undefined;
  let firstOutput: number | undefined;
  let bulkElapsed = 0;
  let releasing!: () => void;
  let held = holdAck;
  const ackGate = new Promise<void>((resolve) => { releasing = resolve; });
  const probes = new Map<number, number>();
  const ownResponses: number[] = [];
  const markers = new ProbeMarkers((sequence) => {
    const submitted = probes.get(sequence);
    if (submitted !== undefined) { const response=performance.now()-submitted;responses.push(response);ownResponses.push(response);probes.delete(sequence); }
  });
  const transport: TerminalTransport = {
    async start(_launch, onOutput, onStatus) {
      output.onmessage = (message) => {
        ledger.rawBinary &&= message instanceof ArrayBuffer;
        onOutput(message);
      };
      status.onmessage = onStatus;
      const started = await invokeTauri("m0_start_fixture", { scenario, backend, onOutput: output, onStatus: status });
      id = started.id;
      return started;
    },
    input: (session, bytes) => invokeTauri("write_terminal_input", { id: session, input: new TextDecoder().decode(bytes) }),
    async ack(session, bytes) {
      if (held) await ackGate;
      await invokeTauri("ack_terminal_output", { id: session, bytes });
      ledger.acknowledge(bytes);
    },
    resize: (session, columns, rows) => invokeTauri("resize_terminal", { id: session, columns, rows }),
    stop: (session) => invokeTauri("stop_terminal", { id: session }),
    cancelAttempt: async () => undefined,
    detach() { output.onmessage = null; status.onmessage = null; },
  };
  let controller: ReturnType<typeof createTerminalController> | undefined;
  const terminal = createTerminalSession(element, false, {
    sendInput: (bytes) => controller?.sendInput(bytes),
    ackOutput: (bytes) => controller?.acknowledge(bytes),
    onResize: (columns, rows) => controller?.resize(columns, rows),
    onParsedOutput: (bytes) => {
      ledger.parsed(bytes); markers.accept(bytes);
      if (markers.ended && bulkElapsed === 0 && firstOutput !== undefined) bulkElapsed = performance.now() - firstOutput;
    },
    onInputError: () => { failure = "input"; },
    onInputOverflow: () => { failure = "input"; },
  });
  controller = createTerminalController({ kind: "local", options: {} }, {
    output: (bytes) => { firstOutput ??= performance.now(); ledger.receive(bytes); terminal.write(bytes); },
    status: (next) => { snapshot = next; },
    failure: () => { failure = "backend"; },
  }, transport);
  return {
    ledger, terminal, markers,
    get id(): TerminalSessionId | undefined { return id; },
    setVisible(visible: boolean): void { element.hidden = !visible; terminal.setVisible(visible); },
    async ready(signal: AbortSignal): Promise<void> {
      await until(() => !!id || !!failure, signal);
      if (failure) throw new BenchmarkError(failure);
    },
    async source() {
      if (!id) throw new BenchmarkError("backend");
      return invokeTauri("m0_source_sample", { id });
    },
    releaseAck(): void { held = false; releasing(); },
    async probe(sequence: number, signal: AbortSignal): Promise<void> {
      if (probes.size >= 32) throw new BenchmarkError("input");
      probes.set(sequence, performance.now());
      await withDeadline(async (bounded) => {
        await controller?.sendInput(new TextEncoder().encode(`probe:${sequence.toString().padStart(6, "0")}\r`));
        await until(() => !probes.has(sequence) || !!failure, bounded);
      }, signal);
      if (failure) throw new BenchmarkError(failure);
    },
    async finish(signal: AbortSignal): Promise<void> {
      await until(() => markers.ended || !!failure, signal, 600_000);
      if (failure) throw new BenchmarkError(failure);
      await controller?.sendInput(new TextEncoder().encode("exit\r"));
      await until(() => snapshot?.state !== "running" && snapshot !== undefined, signal);
      await until(() => ledger.parsedBytes === ledger.bytes && ledger.ackedBytes === ledger.bytes, signal);
      if (snapshot?.state !== "exited" || snapshot.exit_code !== 0) throw new BenchmarkError("backend");
      const source = await this.source();
      if (!source.digest.eof || source.digest.bytes !== ledger.bytes || source.digest.checksum !== ledger.checksum
        || source.snapshot.metrics.input_bytes !== ledger.bytes || source.snapshot.metrics.output_bytes !== ledger.bytes
        || source.snapshot.metrics.max_outstanding_bytes > 1024 * 1024 + 64 * 1024 || !ledger.rawBinary) {
        throw new BenchmarkError("integrity");
      }
    },
    sample() {
      return id ? { id, bytes: ledger.bytes, checksum: ledger.checksum, frames: ledger.frames, raw_binary: ledger.rawBinary,
        parsed_bytes: ledger.parsedBytes, acked_bytes: ledger.ackedBytes, max_pending_bytes: ledger.maxPendingBytes, bulk_elapsed_ms: bulkElapsed, input_response_ms: ownResponses } : undefined;
    },
    async dispose(): Promise<void> {
      held = false; releasing();
      try { await controller?.dispose(); }
      finally {
        try { terminal.dispose(); }
        finally { probes.clear(); element.remove(); }
      }
    },
  };
}

/** Runs the shipped PTY/controller/xterm/Channel path with synthetic fixtures. */
export async function runBenchmarks(host: HTMLElement, signal: AbortSignal, progress: (kind: Run["kind"], round: number) => void): Promise<Report> {
  const transferRound=await invokeTauri("m2_benchmark_round");
  if(transferRound!==null)return runTransferBenchmarks(host,signal,progress,transferRound);
  const mixed = await invokeTauri("m0_benchmark_mixed");
  const report: Report = { schema: 1, viewport: { width: window.innerWidth, height: window.innerHeight, scale: window.devicePixelRatio || 1 }, runs: [] };
  for (const [kind, round] of [["bulk", 0], ["delayed_ack", 0], ["tab_lifecycle", 1], ["tab_lifecycle", 2], ["tab_lifecycle", 3]] as const) {
    const started = performance.now();
    const run: Run = { kind, round, elapsed_ms: 0, renderer_peak: 0, renderers_after: 0, registered_after: 0,
      input_response_ms: [], isolation_input_response_ms: [], terminals: [], failure: null };
    const terminals: ReturnType<typeof openTerminal>[] = [];
    progress(kind, round);
    try {
      const count = kind === "tab_lifecycle" ? 20 : kind === "delayed_ack" && mixed ? 2 : 1;
      for (let index = 0; index < count; index += 1) {
        const { backend, scenario } = terminalPlan(mixed, kind, index);
        const terminal = openTerminal(host, scenario, kind === "delayed_ack" && index === 0, run.input_response_ms, backend);
        terminals.push(terminal);
        await terminal.ready(signal);
        if (index === 0) terminal.setVisible(true);
      }
      if (kind === "delayed_ack") {
        await until(async () => (await terminals[0].source()).snapshot.metrics.pause_count > 0, signal, 30_000);
        if (mixed) {
          for (let probe = 0; probe < 3; probe += 1) {
            const before = run.input_response_ms.length;
            await terminals[1].probe(900000 + probe, signal);
            run.isolation_input_response_ms.push(run.input_response_ms[before]);
          }
        }
        terminals[0].releaseAck();
      }
      for (let index = 0; index < count; index += 1) {
        terminals.forEach((terminal, selected) => terminal.setVisible(selected === index));
        run.renderer_peak = Math.max(run.renderer_peak, terminals.filter((terminal) => terminal.terminal.rendererMounted).length);
        if (run.renderer_peak > 1) throw new BenchmarkError("renderer");
        for (let probe = 0; probe < 3; probe += 1) await terminals[index].probe(index * 3 + probe, signal);
      }
      for (const terminal of terminals) await terminal.finish(signal);
    } catch (error) { run.failure = error instanceof BenchmarkError ? error.code : "backend"; }
    finally {
      // All started controllers are awaited even if one startup/fixture failed.
      const disposal = await Promise.allSettled(terminals.map((terminal) => terminal.dispose()));
      if (disposal.some((result) => result.status === "rejected")) run.failure ??= "cleanup";
      run.renderers_after = terminals.filter((terminal) => terminal.terminal.rendererMounted).length;
      run.registered_after = await invokeTauri("m0_registered_sessions");
      if (run.renderers_after !== 0 || run.registered_after !== 0) run.failure ??= "cleanup";
      run.terminals = terminals.flatMap((terminal) => { const sample = terminal.sample(); return sample ? [sample] : []; });
      run.elapsed_ms = performance.now() - started;
      report.runs.push(run);
    }
    if (signal.aborted) break;
  }
  await invokeTauri("m0_finish_report", { report });
  return report;
}

async function runTransferBenchmarks(host:HTMLElement,signal:AbortSignal,progress:(kind:Run["kind"],round:number)=>void,round:number):Promise<Report>{
  const report:Report={schema:1,viewport:{width:window.innerWidth,height:window.innerHeight,scale:window.devicePixelRatio||1},runs:[]};
  for(const direction of ["upload","download"] as const){
    progress("tab_lifecycle",round);
    const started=performance.now();
    const run:Run={kind:"tab_lifecycle",round,elapsed_ms:0,renderer_peak:0,renderers_after:0,registered_after:0,input_response_ms:[],isolation_input_response_ms:[],terminals:[],failure:null};
    const terminals:ReturnType<typeof openTerminal>[]=[];
    let load:string|undefined;
    try {
      // A visible local terminal and two siblings on one password SSH connection.
      for(const plan of [{backend:"local",scenario:"idle",hold:false},{backend:"ssh",scenario:"bulk",hold:true},{backend:"ssh",scenario:"idle",hold:false}] as const){
        const terminal=openTerminal(host,plan.scenario,plan.hold,run.input_response_ms,plan.backend);
        terminals.push(terminal);await terminal.ready(signal);
      }
      terminals[0].setVisible(true);
      await until(async()=> (await terminals[1].source()).snapshot.metrics.pause_count>0,signal);
      const source=terminals[2].id;if(!source)throw new BenchmarkError("backend");
      load=await invokeTauri("m2_start_load",{terminal:source,direction});
      const active=async():Promise<boolean>=>!!load&&await invokeTauri("m2_load_active",{id:load});
      await until(active,signal,60_000);
      // Every measured probe starts with all three production jobs Running.
      for(const index of [0,2]){
        terminals.forEach(terminal=>terminal.setVisible(false));terminals[index].setVisible(true);
        run.renderer_peak=Math.max(run.renderer_peak,terminals.filter(terminal=>terminal.terminal.rendererMounted).length);
        if(run.renderer_peak!==1)throw new BenchmarkError("renderer");
        for(let probe=0;probe<10;probe++){
          if(!await active())throw new BenchmarkError("integrity");
          const before=run.input_response_ms.length;
          await terminals[index].probe(index*100+probe,signal);
          if(!await active())throw new BenchmarkError("integrity");
          if(index===2)run.isolation_input_response_ms.push(run.input_response_ms[before]);
        }
      }
      terminals[1].releaseAck();
      for(const index of [1,0,2]){
        terminals.forEach(terminal=>terminal.setVisible(false));terminals[index].setVisible(true);
        run.renderer_peak=Math.max(run.renderer_peak,terminals.filter(terminal=>terminal.terminal.rendererMounted).length);
        if(run.renderer_peak!==1)throw new BenchmarkError("renderer");
        if(!await active())throw new BenchmarkError("integrity");
        await terminals[index].probe(500+index,signal);
        if(!await active())throw new BenchmarkError("integrity");
      }
      await invokeTauri("m2_finish_load",{id:load});load=undefined;
      for(const terminal of terminals)await terminal.finish(signal);
    } catch(error){run.failure=error instanceof BenchmarkError?error.code:"backend";}
    finally {
      if(load)await invokeTauri("m2_cancel_load",{id:load}).catch(()=>{run.failure??="cleanup";});
      const disposed=await Promise.allSettled(terminals.map(terminal=>terminal.dispose()));
      if(disposed.some(result=>result.status==="rejected"))run.failure??="cleanup";
      run.renderers_after=terminals.filter(terminal=>terminal.terminal.rendererMounted).length;
      run.registered_after=await invokeTauri("m0_registered_sessions");
      if(run.renderers_after!==0||run.registered_after!==0)run.failure??="cleanup";
      run.terminals=terminals.flatMap(terminal=>{const sample=terminal.sample();return sample?[sample]:[];});
      run.elapsed_ms=performance.now()-started;report.runs.push(run);
    }
    if(signal.aborted||run.failure)break;
  }
  await invokeTauri("m0_finish_report",{report});return report;
}
