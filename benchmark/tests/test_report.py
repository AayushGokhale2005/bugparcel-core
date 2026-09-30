import importlib.util
import unittest
from pathlib import Path


MODULE = Path(__file__).parents[1] / "scripts" / "report.py"
SPEC = importlib.util.spec_from_file_location("benchmark_report", MODULE)
report = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(report)


class BenchmarkReportTests(unittest.TestCase):
    def setUp(self):
        self.cases = {"target_cases": 24, "cases": [{"id": "api-001", "eligible": True}]}

    def test_reports_genuine_and_false_fixes_separately(self):
        runs = [
            {"run_id": "b", "case_id": "api-001", "arm": "baseline", "outcome": "false_fix", "acceptance_artifact": "artifacts/b"},
            {"run_id": "p", "case_id": "api-001", "arm": "parcel", "outcome": "verified_fix", "acceptance_artifact": "artifacts/p"},
        ]
        result = report.report(self.cases, runs)
        self.assertEqual(result["arms"]["baseline"]["false_fix_rate"], 1.0)
        self.assertEqual(result["arms"]["parcel"]["fix_rate"], 1.0)

    def test_does_not_measure_an_empty_study(self):
        result = report.report(self.cases, [])
        self.assertFalse(result["arms"]["baseline"]["measured"])
        self.assertIsNone(result["arms"]["parcel"]["fix_rate"])

    def test_rejects_runs_for_ineligible_cases(self):
        with self.assertRaisesRegex(ValueError, "not eligible"):
            report.report(self.cases, [{"run_id": "x", "case_id": "api-404", "arm": "parcel", "outcome": "verified_fix", "acceptance_artifact": "a"}])


if __name__ == "__main__":
    unittest.main()
