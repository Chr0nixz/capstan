/** URL-style SSH authority. An "@" or "%" inside the user name is percent-encoded so the
 * host part stays unambiguous (`alice%40corp@host:22`), and IPv6 literals are bracketed. */
export function sshAuthority(username: string | null, host: string, port: number): string {
  const user = username === null ? "" : `${username.replace(/%/gu, "%25").replace(/@/gu, "%40")}@`;
  const address = host.includes(":") && !host.startsWith("[") ? `[${host}]` : host;
  return `${user}${address}:${port}`;
}
