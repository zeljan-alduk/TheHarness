import unittest
from app.report import report_line
from app.export import export_row


class T(unittest.TestCase):
    def test_report(self):
        self.assertEqual(report_line("x", 1234), "x: $12.34")

    def test_export(self):
        self.assertEqual(export_row(-5), {"amount": "-$0.05"})
