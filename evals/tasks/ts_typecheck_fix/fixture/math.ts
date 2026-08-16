export function avg(xs: number[]): string {
  let s = 0;
  for (const x of xs) s += x;
  return String(s / xs.length);
}
