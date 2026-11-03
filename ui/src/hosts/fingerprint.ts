export type FingerprintChunk = { text: string; changed: boolean };

/**
 * Splits a fingerprint into short groups for comparison by eye. When `other`
 * is given, groups that differ from it at the same position are marked.
 */
export function fingerprintChunks(value: string, other: string | null): FingerprintChunk[] {
  const prefix = /^[A-Z0-9]+:/u.exec(value)?.[0] ?? "";
  const chunks: FingerprintChunk[] = prefix ? [{ text: prefix, changed: other !== null && !other.startsWith(prefix) }] : [];
  for (let index = prefix.length; index < value.length; index += 4) {
    const text = value.slice(index, index + 4);
    chunks.push({ text, changed: other !== null && other.slice(index, index + 4) !== text });
  }
  return chunks;
}

/** The OpenSSH server key file that matches a host key algorithm, for `ssh-keygen -lf`. */
export function hostKeyFile(algorithm: string): string {
  const kind = algorithm.startsWith("ecdsa-") ? "ecdsa"
    : algorithm === "ssh-rsa" || algorithm.startsWith("rsa-sha2-") ? "rsa"
      : algorithm === "ssh-dss" ? "dsa" : "ed25519";
  return `/etc/ssh/ssh_host_${kind}_key.pub`;
}
