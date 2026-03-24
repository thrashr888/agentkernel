#!/usr/bin/env python3
import json
import sys
from pathlib import Path

path = Path(sys.argv[1] if len(sys.argv) > 1 else "autoresearch/latest-report.json")
report = json.loads(path.read_text())

print(f"report: {path}")
print(f"primary_metric: {report['primary_metric']}")
print(f"total_score: {report['total_score']:.2f}")
print()
print("backend          startup_avg_ms  exec_avg_ms  lifecycle_ms  total_avg_ms  throughput/s  score")
print("---------------  --------------  -----------  ------------  ------------  ------------  ------")
for backend in report.get("backends", []):
    lifecycle = backend.get('lifecycle_total', {}).get('avg_ms', 0.0)
    print(
        f"{backend['backend']:<15}  "
        f"{backend['startup']['avg_ms']:>14.2f}  "
        f"{backend['exec']['avg_ms']:>11.2f}  "
        f"{lifecycle:>12.2f}  "
        f"{backend['total']['avg_ms']:>12.2f}  "
        f"{backend['throughput_per_second']:>12.2f}  "
        f"{backend['scores']['total_score']:>6.2f}"
    )
