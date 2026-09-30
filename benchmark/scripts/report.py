#!/usr/bin/env python3
"""Compute transparent BugParcel benchmark rates from reviewed run records."""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path
from typing import Any

ARMS = ("baseline", "parcel")
OUTCOMES = {"verified_fix", "false_fix", "unresolved", "infra_failure", "excluded"}


def load_json(path: Path) -> dict[str, Any]:
    with path.open(encoding="utf-8") as file:
        return json.load(file)


def load_runs(path: Path) -> list[dict[str, Any]]:
    if not path.exists():
        return []
    with path.open(encoding="utf-8") as file:
        return [json.loads(line) for line in file if line.strip()]


def validate(cases: dict[str, Any], runs: list[dict[str, Any]]) -> None:
    case_ids = {case["id"] for case in cases.get("cases", []) if case.get("eligible")}
    seen: set[tuple[str, str, str]] = set()
    for run in runs:
        arm = run.get("arm")
        outcome = run.get("outcome")
        case_id = run.get("case_id")
        run_id = run.get("run_id")
        if arm not in ARMS:
            raise ValueError(f"{run_id}: arm must be one of {ARMS}")
        if outcome not in OUTCOMES:
            raise ValueError(f"{run_id}: invalid outcome {outcome!r}")
        if case_id not in case_ids:
            raise ValueError(f"{run_id}: case {case_id!r} is not eligible")
        if not run.get("acceptance_artifact"):
            raise ValueError(f"{run_id}: missing acceptance_artifact")
        key = (case_id, arm, run.get("trial", "default"))
        if key in seen:
            raise ValueError(f"duplicate run for {key}")
        seen.add(key)


def arm_report(runs: list[dict[str, Any]], arm: str) -> dict[str, Any]:
    outcomes = Counter(run["outcome"] for run in runs if run["arm"] == arm)
    denominator = sum(outcomes[outcome] for outcome in ("verified_fix", "false_fix", "unresolved"))
    return {
        "measured": denominator > 0,
        "runs": sum(outcomes.values()),
        "genuine_fixes": outcomes["verified_fix"] if denominator else None,
        "false_fixes": outcomes["false_fix"] if denominator else None,
        "unresolved": outcomes["unresolved"],
        "infra_failures": outcomes["infra_failure"],
        "excluded": outcomes["excluded"],
        "denominator": denominator,
        "fix_rate": round(outcomes["verified_fix"] / denominator, 4) if denominator else None,
        "false_fix_rate": round(outcomes["false_fix"] / denominator, 4) if denominator else None,
    }


def report(cases: dict[str, Any], runs: list[dict[str, Any]]) -> dict[str, Any]:
    validate(cases, runs)
    eligible = sum(bool(case.get("eligible")) for case in cases.get("cases", []))
    return {
        "schema_version": 1,
        "study_status": "measured" if runs else "curating_corpus",
        "target_cases": cases.get("target_cases", 24),
        "eligible_cases": eligible,
        "arms": {arm: arm_report(runs, arm) for arm in ARMS},
        "notes": [
            "Rates use independently reviewed acceptance outcomes.",
            "Infrastructure failures and exclusions are preserved but excluded from rate denominators.",
        ],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--cases", type=Path, default=Path("cases.json"))
    parser.add_argument("--runs", type=Path, default=Path("runs.jsonl"))
    parser.add_argument("--output", type=Path, default=Path("results.json"))
    args = parser.parse_args()
    results = report(load_json(args.cases), load_runs(args.runs))
    args.output.write_text(json.dumps(results, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
