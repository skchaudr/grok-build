"""Renders the writer's returned packet unchanged through the existing task() helper.

Reads inspector-focus.packet.md (SLICE_ID, SOURCE_RANGES, SPEC sections) and
builds one task. The packet text is not edited; only split on its headings.
"""
import importlib.util
import re
from pathlib import Path

HERE = Path(__file__).resolve().parent
_s = importlib.util.spec_from_file_location("base", HERE.parents[1] / "acpx-disclosure-slices.py")
base = importlib.util.module_from_spec(_s)
_s.loader.exec_module(base)
REPOS, SYSTEM = base.REPOS, base.SYSTEM

text = (HERE / "inspector-focus.packet.md").read_text()
def section(name, nxt):
    m = re.search(rf"^{name}:\s*\n(.*?)(?=^{nxt}:|\Z)", text, re.M | re.S)
    return m.group(1).strip()
slice_id = section("SLICE_ID", "SOURCE_RANGES")
ranges = [l.strip() for l in section("SOURCE_RANGES", "SPEC").splitlines() if l.strip()]
spec = section("SPEC", "RANGE_NOTES")
TASKS = [base.task(slice_id, ranges, spec)]
