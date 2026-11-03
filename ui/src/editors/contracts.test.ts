import { describe, expect, it } from "vitest";
import fixture from "../../../docs/agent/fixtures/M2-editor.json";
import { decodeRemoteEditSnapshot } from "../sftp/contracts";

describe("remote edit metadata contract", () => {
  it("retains exact revisions and represents recovered copies without remote authority", () => {
    const snapshot = decodeRemoteEditSnapshot(fixture);
    expect(snapshot.revision).toBe("9007199254740993");
    expect(snapshot.sessions[1]).toMatchObject({ remotePath: null, hostId: null, recovered: true });
    expect(JSON.stringify(snapshot)).not.toMatch(/executable|localPath|bytes|content/u);
  });
  it("rejects malformed states, errors, duplicate IDs, unbounded lists and private fields", () => {
    for (const invalid of [
      { ...fixture, revision: 9007199254740992 },
      { ...fixture, sessions: [...fixture.sessions, fixture.sessions[0]] },
      { ...fixture, sessions: Array.from({ length: 9 }, () => fixture.sessions[0]) },
      ...[{ state: "uploading" }, { error: "server said no" }, { localPath: "C:\\private" }, { name: "../escape" }, { remotePath: "relative" }, { hostId: "host" }].map((fields) => ({ ...fixture, sessions: [{ ...fixture.sessions[0], ...fields }] })),
      ...[
        Array.from({ length: 9 }, () => fixture.sessions[0].remoteTemporaries[0]),
        [fixture.sessions[0].remoteTemporaries[0], fixture.sessions[0].remoteTemporaries[0]],
        [{ ...fixture.sessions[0].remoteTemporaries[0], path: "relative" }],
        [{ ...fixture.sessions[0].remoteTemporaries[0], state: "delete_anyway" }],
        [{ ...fixture.sessions[0].remoteTemporaries[0], hash: "private-proof" }],
      ].map((remoteTemporaries) => ({ ...fixture, sessions: [{ ...fixture.sessions[0], remoteTemporaries }] })),
    ]) expect(() => decodeRemoteEditSnapshot(invalid)).toThrow("sftp_contract_invalid");
  });
});
