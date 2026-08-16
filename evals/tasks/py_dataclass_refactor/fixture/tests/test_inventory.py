import unittest
from inventory import make_item, inventory_total


class T(unittest.TestCase):
    def test_total(self):
        items = [make_item("a", 2, 1.5), make_item("b", 1, 4.0)]
        self.assertAlmostEqual(inventory_total(items), 7.0)
