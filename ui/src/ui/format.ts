/** Binary units with one decimal, computed in BigInt so u64 counters stay exact. */
export function formatBytes(value: string): string {
  const bytes = BigInt(value);
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
  let unit = 0, scale = 1n;
  while (bytes >= scale * 1024n && unit < units.length - 1) { scale *= 1024n; unit++; }
  return `${bytes / scale}${unit > 0 ? `.${(bytes % scale) * 10n / scale}` : ""} ${units[unit]}`;
}
