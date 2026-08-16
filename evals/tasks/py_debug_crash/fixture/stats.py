import json, sys

data = json.load(open(sys.argv[1]))
ages = [u["age"] for u in data]
print(f"average age: {sum(ages) / len(ages):.2f}")
