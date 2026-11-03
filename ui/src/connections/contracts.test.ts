import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M3-contract.json";
import { acceptsConnectionSnapshot, connectionStateChangedEvent, decodeConnectionSnapshot } from "./contracts";

describe("connection lifecycle contract", () => {
  it("preserves a u64 generation exactly and validates all shared transition examples", () => {
    const snapshot = decodeConnectionSnapshot(fixture.connectionSnapshot);
    expect(snapshot.generation).toBe("9007199254740993");
    expect(connectionStateChangedEvent).toBe(fixture.eventNames.connectionStateChanged);
    for (const transition of fixture.connectionTransitions) {
      expect(acceptsConnectionSnapshot(decodeConnectionSnapshot(transition.current), decodeConnectionSnapshot(transition.next)))
        .toBe(transition.accepted);
    }
  });

  it.each(fixture.connectionSnapshotCases)("matches Rust acceptance for $name", ({ patch, accepted }) => {
    const decode = () => decodeConnectionSnapshot({ ...fixture.connectionSnapshot, ...patch });
    if (accepted) expect(decode()).toMatchObject(patch);
    else expect(decode).toThrow("connection_contract_invalid");
  });

  it("rejects malformed snapshots and dynamic error messages", () => {
    for (const value of [
      { ...fixture.connectionSnapshot, generation: 1 },
      { ...fixture.connectionSnapshot, generation: "01" },
      { ...fixture.connectionSnapshot, generation: "18446744073709551616" },
      { ...fixture.connectionSnapshot, state: "retrying" },
      { ...fixture.connectionSnapshot, errorCode: "secret from server" },
      { ...fixture.connectionSnapshot, detail: "server message" },
    ]) expect(() => decodeConnectionSnapshot(value)).toThrow("connection_contract_invalid");
  });
});
