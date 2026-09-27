#!/usr/bin/env python3
"""Print the accessibility tree that a running pigment-studio exposes on AT-SPI.

Linux only. Needs the system PyGObject with the Atspi 2.0 typelib (not
installable from PyPI), and accessibility enabled for the session. AccessKit
registers only when a screen reader is announced, so for a check without a
screen reader running:

    busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status IsEnabled b true
    busctl --user set-property org.a11y.Bus /org/a11y/bus org.a11y.Status ScreenReaderEnabled b true
    ./target/release/pigment-studio &  sleep 5;  python3 scripts/atspi-dump.py;  kill %1
    # then set both back to false (their usual value)
"""
import sys, time
import gi
gi.require_version("Atspi", "2.0")
from gi.repository import Atspi

def walk(node, depth, out, limit=400):
    if len(out) > limit or depth > 25:
        return
    try:
        role = node.get_role_name(); name = node.get_name() or ""
        states = node.get_state_set()
        focusable = states.contains(Atspi.StateType.FOCUSABLE)
        labelled = ""
        try:
            for rel in node.get_relation_set():
                if rel.get_relation_type() == Atspi.RelationType.LABELLED_BY:
                    labelled = " labelled-by=" + ",".join((rel.get_target(i).get_name() or "?") for i in range(rel.get_n_targets()))
        except Exception:
            pass
        out.append(f"{'  '*depth}{role}: {name!r}{' [focusable]' if focusable else ''}{labelled}")
        for i in range(node.get_child_count()):
            walk(node.get_child_at_index(i), depth + 1, out, limit)
    except Exception as e:
        out.append(f"{'  '*depth}<error {e}>")

deadline = time.time() + 15
while time.time() < deadline:
    desktop = Atspi.get_desktop(0)
    apps = [desktop.get_child_at_index(i) for i in range(desktop.get_child_count())]
    target = [a for a in apps if a and "pigment" in (a.get_name() or "").lower()]
    if target:
        out = []
        walk(target[0], 0, out)
        if len(out) > 5:
            print("\n".join(out)); sys.exit(0)
    time.sleep(0.5)
print("no Pigment Prose application on the AT-SPI bus; apps:", [a.get_name() for a in apps if a])
sys.exit(1)
