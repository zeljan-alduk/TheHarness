import time, random
from dedupe import dedupe

random.seed(1)
data = [random.randint(0, 50000) for _ in range(200000)]
t = time.time()
r = dedupe(data)
dt = time.time() - t
seen = set(); expect = []
for x in data:
    if x not in seen:
        seen.add(x); expect.append(x)
print("ok" if r == expect and dt < 2.0 else f"fail (dt={dt:.2f}, correct={r == expect})")
