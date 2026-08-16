def format_money(cents: int) -> str:
    sign = "-" if cents < 0 else ""
    cents = abs(cents)
    return f"{sign}${cents // 100}.{cents % 100:02d}"


def export_row(cents: int) -> dict:
    return {"amount": format_money(cents)}
