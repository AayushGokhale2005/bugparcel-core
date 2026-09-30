# BugParcel agent benchmark

This is an open, preregistered-style benchmark harness for one hypothesis:

> Giving a coding agent a BugParcel should increase genuine fixes and reduce
> false fixes for reproducible FastAPI/PostgreSQL failures.

It is **not a performance claim**. `results.json` intentionally contains no
measured rates until cases are curated and runs are independently checked.

## Corpus

The target corpus is 24 public FastAPI/PostgreSQL bugs (acceptable range:
20–30). A case becomes eligible only when it has all of the following:

1. A public source repository and immutable base commit.
2. A repeatable failing command pinned by lockfile or container image.
3. A committed, independently runnable acceptance command that proves the
   intended behavior after a patch.
4. A review that confirms fixtures contain no private production data.
5. A BugParcel captured before either agent arm receives the task.

Each accepted case goes in `cases.json`. Do not add synthetic tasks, resolved
issues without a reproducible pre-fix commit, or a task whose acceptance check
can be changed by the candidate patch.

## Protocol

Every eligible case is run in two arms with the same model, tools, time limit,
and base checkout:

| Arm | Agent input |
| --- | --- |
| `baseline` | Issue summary, repository checkout, and normal tool access. |
| `parcel` | The same input plus the captured BugParcel contract and its isolated replay workflow. |

The agent may edit only its supplied detached checkout. The evaluator then
applies the candidate diff to a fresh checkout at the source commit and runs
the immutable acceptance command. A fix is genuine only when that command
passes. A patch that hides the original failure but fails acceptance is a
`false_fix`.

Run order is randomized per case. Keep the seed, model version, tool version,
wall-clock budget, prompt text, patch, replay log, and acceptance output with
each outcome record.

## Reporting

```sh
python3 scripts/report.py --cases cases.json --runs runs.jsonl --output results.json
```

The report excludes `infra_failure` records from the denominator, but preserves
their count. It never converts missing runs into failures or a rate. The landing
page must show `Not measured` until each compared metric has a denominator.

Run the harness tests with:

```sh
python3 -m unittest discover -s tests -t . -v
```

## Review checklist

- Two reviewers approve case eligibility and acceptance immutability.
- One reviewer who did not run the agent classifies the outcome.
- A false fix is reported separately from an unresolved task.
- Publish raw, non-secret run artifacts and the exact reporting command.
