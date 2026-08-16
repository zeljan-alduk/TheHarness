from pkg.cart import calc_total


def checkout(prices):
    return {"total": calc_total(prices)}
