from merge import deep_merge

a = {"x": 1, "n": {"a": 1, "b": 2}, "l": [1, 2], "keep": "yes"}
b = {"x": 9, "n": {"b": 3, "c": 4}, "l": [3]}
out = deep_merge(a, b)
assert out == {"x": 9, "n": {"a": 1, "b": 3, "c": 4}, "l": [1, 2, 3], "keep": "yes"}, out
assert a == {"x": 1, "n": {"a": 1, "b": 2}, "l": [1, 2], "keep": "yes"}, "a was mutated"
assert b == {"x": 9, "n": {"b": 3, "c": 4}, "l": [3]}, "b was mutated"
assert deep_merge({}, {}) == {}
assert deep_merge({"a": {"b": 1}}, {"a": 2}) == {"a": 2}
print("ok")
