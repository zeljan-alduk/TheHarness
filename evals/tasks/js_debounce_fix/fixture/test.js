const { debounce } = require("./debounce");

let calls = [];
const obj = {
  name: "obj",
  handler: debounce(function (v) { calls.push([this && this.name, v]); }, 40),
};

obj.handler(1);
obj.handler(2);
obj.handler(3);

setTimeout(() => {
  if (calls.length !== 1) { console.error("expected 1 call, got", calls.length, calls); process.exit(1); }
  const [thisName, v] = calls[0];
  if (v !== 3) { console.error("expected the latest argument 3, got", v); process.exit(1); }
  if (thisName !== "obj") { console.error("expected `this` to be preserved, got", thisName); process.exit(1); }
  console.log("ok");
}, 120);

setTimeout(() => { if (calls.length !== 0) { console.error("fired too early:", calls); process.exit(1); } }, 10);
