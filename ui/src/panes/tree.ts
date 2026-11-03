import type { PaneId, PaneNode } from "./contracts";

export type PaneLeaf = Extract<PaneNode, { kind: "pane" }>;
export type SplitDirection = Extract<PaneNode, { kind: "split" }>["direction"];

export function paneLeaves(root: PaneNode): PaneLeaf[] {
  return root.kind === "pane" ? [root] : [...paneLeaves(root.first), ...paneLeaves(root.second)];
}

export function findPane(root: PaneNode, id: PaneId): PaneLeaf | undefined {
  return root.kind === "pane" ? root.paneId === id ? root : undefined : findPane(root.first, id) ?? findPane(root.second, id);
}

export function replacePane(root: PaneNode, id: PaneId, replacement: PaneNode): PaneNode {
  if (root.kind === "pane") return root.paneId === id ? replacement : root;
  const first = replacePane(root.first, id, replacement), second = replacePane(root.second, id, replacement);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

/** Collapses only the removed leaf's ancestors, preserving every surviving launch. */
export function removePane(root: PaneNode, id: PaneId): PaneNode | null {
  if (root.kind === "pane") return root.paneId === id ? null : root;
  const first = removePane(root.first, id), second = removePane(root.second, id);
  if (!first) return second;
  if (!second) return first;
  return first === root.first && second === root.second ? root : { ...root, first, second };
}

export function splitId(root: Extract<PaneNode, { kind: "split" }>): string {
  return `${paneLeaves(root.first)[0].paneId}/${paneLeaves(root.second)[0].paneId}`;
}

export function resizeSplit(root: PaneNode, id: string, ratio: number): PaneNode {
  if (root.kind === "pane" || !Number.isFinite(ratio)) return root;
  if (splitId(root) === id) {
    const next = Math.min(0.95, Math.max(0.05, ratio));
    return Math.abs(next - root.ratio) < 0.000001 ? root : { ...root, ratio: next };
  }
  const first = resizeSplit(root.first, id, ratio), second = resizeSplit(root.second, id, ratio);
  return first === root.first && second === root.second ? root : { ...root, first, second };
}
