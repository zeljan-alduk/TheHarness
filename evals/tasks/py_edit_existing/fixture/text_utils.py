import re


def slugify(text: str) -> str:
    """Turn arbitrary text into a URL slug."""
    text = re.sub(r"[^A-Za-z0-9]+", "-", text)
    return text
