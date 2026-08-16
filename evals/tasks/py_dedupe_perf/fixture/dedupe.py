def dedupe(items):
    out = []
    for x in items:
        if x not in out:
            out.append(x)
    return out
