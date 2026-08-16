def format_money(cents: int) -> str:
    sign = "-" if cents < 0 else ""
    cents = abs(cents)
    return f"{sign}${cents // 100}.{cents % 100:02d}"


def report_line(name: str, cents: int) -> str:
    return f"{name}: {format_money(cents)}"
