/** English count phrase such as `1 file` or `2 files`. Chinese copy has no plural forms. */
export function pluralize(count: number, one: string, other: string): string {
  return `${count} ${count === 1 ? one : other}`;
}
