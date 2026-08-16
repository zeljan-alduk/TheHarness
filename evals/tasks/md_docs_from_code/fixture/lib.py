def add(a: int, b: int) -> int:
    """Return the sum of a and b."""
    return a + b


def _helper(x):
    return x * 2


def slugify(text: str) -> str:
    """Lower-case and dash-join words."""
    return "-".join(text.lower().split())


def clamp(x: float, lo: float, hi: float) -> float:
    """Clamp x into [lo, hi]."""
    return max(lo, min(hi, x))
