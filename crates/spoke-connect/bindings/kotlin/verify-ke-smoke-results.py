#!/usr/bin/env python3
"""Gate the Kotlin KE smoke JUnit XML against the committed expectations.

Run from `crates/spoke-connect/bindings/kotlin`:

    python3 verify-ke-smoke-results.py build/test-results/test

A green Gradle run that executed nothing is not a pass: this gate fails unless
the committed `Smoke/PortsLoopbackFfiPairTest.kt` declares exactly the ten
`keRemote_*` methods and every one of them was executed once with no failure /
error / skipped entry, while every other executed testcase passed. Prints
`Kotlin KE smoke: PASS (10/10)` on success.
"""

from __future__ import annotations

import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import NoReturn

KE_CLASS = "PortsLoopbackFfiPairTest"
KE_PREFIX = "keRemote_"
KE_SOURCE = Path("Smoke") / f"{KE_CLASS}.kt"
PINNED_KE_COUNT = 10


def fail(reason: str) -> NoReturn:
    print(f"Kotlin KE smoke: FAIL - {reason}", file=sys.stderr)
    raise SystemExit(1)


def committed_ke_methods(project_dir: Path) -> list[str]:
    """The `@Test`-annotated `keRemote_*` methods in the committed smoke source."""
    source = project_dir / KE_SOURCE
    if not source.is_file():
        fail(f"committed smoke source not found: {source}")
    lines = source.read_text(encoding="utf-8").splitlines()
    names: list[str] = []
    for index, line in enumerate(lines):
        match = re.match(rf"\s*fun\s+({re.escape(KE_PREFIX)}\w+)\s*\(", line)
        if match is None:
            continue
        previous = index - 1
        while previous >= 0 and not lines[previous].strip():
            previous -= 1
        if previous >= 0 and lines[previous].strip().startswith("@Test"):
            names.append(match.group(1))
    return names


def load_cases(results_dir: Path) -> list[tuple[str, str, ET.Element]]:
    """(class, method, testcase element) for every reported testcase."""
    xml_files = sorted(results_dir.glob("TEST-*.xml"))
    if not xml_files:
        fail(f"no JUnit XML under {results_dir} - the smoke executed zero tests")
    cases: list[tuple[str, str, ET.Element]] = []
    for xml_file in xml_files:
        try:
            suite = ET.parse(xml_file).getroot()
        except ET.ParseError as exc:
            fail(f"unparsable JUnit XML {xml_file}: {exc}")
        suite_name = suite.get("name", "")
        for case in suite.iter("testcase"):
            # Gradle/JUnit5 report `keRemote_x()` / `keRemote_x(String)`.
            method = case.get("name", "").split("(")[0].strip()
            class_name = (case.get("classname") or suite_name).split(".")[-1]
            cases.append((class_name, method, case))
    if not cases:
        fail(f"no testcase entries under {results_dir} - the smoke executed zero tests")
    return cases


def main(argv: list[str]) -> int:
    if len(argv) > 2:
        fail(f"usage: {Path(argv[0]).name} [junit-xml-dir]")
    project_dir = Path(__file__).resolve().parent
    results_dir = Path(argv[1] if len(argv) == 2 else "build/test-results/test")
    if not results_dir.is_dir():
        fail(f"JUnit XML directory not found: {results_dir}")

    expected = committed_ke_methods(project_dir)
    if len(expected) != PINNED_KE_COUNT:
        fail(
            f"{KE_SOURCE} declares {len(expected)} {KE_PREFIX}* methods; "
            f"the pinned expectation is {PINNED_KE_COUNT}"
        )
    source_duplicates = sorted({name for name in expected if expected.count(name) > 1})
    if source_duplicates:
        fail(f"duplicate {KE_PREFIX}* method names in {KE_SOURCE}: {', '.join(source_duplicates)}")

    per_class: dict[str, dict[str, int]] = {}
    ke_runs: dict[str, int] = {}
    failed_other: list[str] = []

    for class_name, method, case in load_cases(results_dir):
        counts = per_class.setdefault(class_name, {"executed": 0, "failed": 0, "skipped": 0})
        counts["executed"] += 1
        is_ke = class_name == KE_CLASS and method.startswith(KE_PREFIX)
        failed = case.find("failure") is not None or case.find("error") is not None
        skipped = case.find("skipped") is not None
        if failed:
            counts["failed"] += 1
        if skipped:
            counts["skipped"] += 1

        if is_ke:
            ke_runs[method] = ke_runs.get(method, 0) + 1
            if failed:
                fail(f"{class_name}.{method} failed")
            if skipped:
                fail(f"{class_name}.{method} was skipped")
        elif failed or skipped:
            failed_other.append(f"{class_name}.{method}" + (" (skipped)" if skipped else ""))

    if failed_other:
        fail("executed cases that did not pass: " + ", ".join(sorted(failed_other)))

    repeated = sorted(method for method, runs in ke_runs.items() if runs > 1)
    if repeated:
        fail("duplicate executions of: " + ", ".join(repeated))
    missing = sorted(set(expected) - set(ke_runs))
    if missing:
        fail(f"{KE_PREFIX}* cases that did not execute: " + ", ".join(missing))
    unexpected = sorted(set(ke_runs) - set(expected))
    if unexpected:
        fail(f"unexpected {KE_PREFIX}* cases executed: " + ", ".join(unexpected))

    for class_name in sorted(per_class):
        counts = per_class[class_name]
        print(
            f"  {class_name}: executed={counts['executed']} "
            f"failed={counts['failed']} skipped={counts['skipped']}"
        )
    total = {key: sum(counts[key] for counts in per_class.values()) for key in ("executed", "failed", "skipped")}
    print(
        f"  {KE_CLASS} {KE_PREFIX}*: executed={len(ke_runs)} failed=0 skipped=0 "
        f"(all classes: executed={total['executed']} failed={total['failed']} skipped={total['skipped']})"
    )
    print(f"Kotlin KE smoke: PASS ({len(ke_runs)}/{PINNED_KE_COUNT})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
