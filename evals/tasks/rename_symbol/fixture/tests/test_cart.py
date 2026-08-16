import unittest
from pkg.checkout import checkout


class T(unittest.TestCase):
    def test_total(self):
        self.assertEqual(checkout([1, 2, 3])["total"], 6)
