import { avg } from "./math.ts";
const a: number = avg([1, 2, 3, 4] as any) as any;
console.log(`avg=${(a as any).toFixed ? (a as number).toFixed(1) : a}`);
