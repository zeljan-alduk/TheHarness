def make_item(name, qty, price):
    return {"name": name, "qty": qty, "price": price}


def inventory_total(items):
    return sum(i["qty"] * i["price"] for i in items)
