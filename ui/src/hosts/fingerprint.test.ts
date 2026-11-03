import { describe, expect, it } from "vitest";
import { fingerprintChunks, hostKeyFile } from "./fingerprint";

describe("host key fingerprints", () => {
  it("groups a fingerprint and marks the groups that changed", () => {
    const known = "SHA256:abcdEFGHijkl";
    expect(fingerprintChunks(known, null).map((chunk) => chunk.text)).toEqual(["SHA256:", "abcd", "EFGH", "ijkl"]);
    expect(fingerprintChunks("SHA256:abcdXFGHijkl", known).map((chunk) => chunk.changed)).toEqual([false, false, true, false]);
    expect(fingerprintChunks("MD5:ab", known)[0]).toEqual({ text: "MD5:", changed: true });
  });

  it("names the server key file for each algorithm", () => {
    expect(hostKeyFile("ssh-ed25519")).toBe("/etc/ssh/ssh_host_ed25519_key.pub");
    expect(hostKeyFile("ecdsa-sha2-nistp256")).toBe("/etc/ssh/ssh_host_ecdsa_key.pub");
    expect(hostKeyFile("rsa-sha2-512")).toBe("/etc/ssh/ssh_host_rsa_key.pub");
    expect(hostKeyFile("ssh-rsa")).toBe("/etc/ssh/ssh_host_rsa_key.pub");
  });
});
