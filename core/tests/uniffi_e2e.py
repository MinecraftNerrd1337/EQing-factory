"""End-to-end test of the uniFFI layer, from Python (same generated bindings
as for Swift). Run with: tests/run_e2e.sh"""
import glob, json, os, shutil, sys, tempfile

sys.path.insert(0, sys.argv[1])
import eqing_core as pc  # noqa: E402

CUR = sys.argv[2]
ARIA = f"{CUR}/measurements/in-ear/Moondrop Aria.csv"
ZERO = f"{CUR}/measurements/in-ear/Truthear x Crinacle Zero.csv"
HARMAN = f"{CUR}/targets/Harman in-ear 2019.csv"

checks = 0
def ok(cond, msg):
    global checks
    if not cond:
        raise AssertionError(msg)
    checks += 1

d = tempfile.mkdtemp()
core = pc.AppCore(d)
s1 = core.active_session()
ok(len(core.sessions(False)) == 1, "one session at startup")
tab = core.active_tab(s1)
ok(tab is not None, "one tab at startup")

# folders and sessions
fo = core.create_folder("IEMs", None)
sub = core.create_folder("Chi-Fi", fo)
s2 = core.create_session("Client B", sub)
ok(any(s.id == s2 and s.folder == sub for s in core.sessions(False)), "session in a subfolder")
try:
    core.move_folder(fo, sub); ok(False, "folder cycle accepted")
except pc.CoreError.Invalid:
    ok(True, "")
core.delete_folder(sub)
ok(next(s for s in core.sessions(False) if s.id == s2).folder == fo, "contents moved up to the parent")

# measurement, target, autoeq
m = core.add_curve_from_file(tab, ARIA, pc.Role.MEASUREMENT)
t = core.add_curve_from_file(tab, HARMAN, pc.Role.TARGET)
tv = core.tab(tab)
ok(tv.measurement == m and tv.target == t, "roles")
ok(tv.curves[0].color != tv.curves[1].color, "distinct automatic colors")
ok(tv.bands == 20 and tv.mode == pc.Mode.IN_EAR, "defaults: 20 bands, mode i")
core.set_tab_params(tab, 10, 3000, pc.Mode.IN_EAR)
info = core.run_autoeq(tab)
tv = core.tab(tab)
ok(len(tv.filters) == 10 and all(f.origin == pc.FilterOrigin.AUTO for f in tv.filters), "10 auto filters")
ok(tv.filters[0].kind == pc.FilterKind.LOW_SHELF and tv.filters[1].kind == pc.FilterKind.HIGH_SHELF, "LSC then HSC")
ok(abs(tv.filters[0].f0 - 50.8) < 0.1, f"first filter expected at 50.8 Hz: {tv.filters[0].f0}")
ok(tv.preamp <= 0, "negative preamp")
plot = core.plot(tab, 400)
keys = [s.key for s in plot.series]
ok("result" in keys and "eq" in keys, "result and EQ series")
res = next(s for s in plot.series if s.key == "result").values
tgt = next(s for s in plot.series if s.key == t).values
mid = [abs(a - b) for a, b, f in zip(res, tgt, plot.freqs) if 100 < f < 10000]
ok(sum(mid) / len(mid) < 1.5, f"result aligned to the target (mean deviation {sum(mid)/len(mid):.2f} dB)")

# manual filters, undo/redo
fm = core.add_filter(tab, pc.FilterKind.PEAK, 1000, 2.0, 1.0)
ok(core.tab(tab).filters[-1].origin == pc.FilterOrigin.MANUAL, "manual filter")
for g in (2.5, 3.0, 3.5):  # dragging a field: grouped into one step
    core.update_filter(tab, fm, pc.FilterKind.PEAK, 1000, g, 1.0, True)
ok(core.undo(tab) and core.tab(tab).filters[-1].gain == 2.0, "grouped undo")
ok(core.redo(tab) and core.tab(tab).filters[-1].gain == 3.5, "redo")
core.run_autoeq(tab)
fl = core.tab(tab).filters
ok(len(fl) == 11 and fl[-1].id == fm, "the manual filter survives a new run")
core.remove_filter(tab, fm)
ok(core.undo(tab) and any(f.id == fm for f in core.tab(tab).filters), "undo of a removal")

# exports / imports
apo = core.export_eq(tab, pc.ExportFormat.PARAMETRIC)
ok(apo.startswith("Preamp:") and apo.count("Filter ") == 11, "parametric export")
ge = core.export_eq(tab, pc.ExportFormat.GRAPHIC_EQ)
ok(ge.startswith("GraphicEQ: 20 ") and ge.count(";") == 126, "GraphicEQ export (127 points)")
csv = core.export_curve_csv(tab, pc.CurveRef.RESULT())
ok(csv.startswith("frequency,raw\n"), "CSV export of the result")
t2 = core.create_tab(s1, "Import")
rep = core.import_filters_text(t2, apo, True)
ok(rep.count == 11 and not rep.skipped, "re-import of the export")
ok(all(f.origin == pc.FilterOrigin.IMPORTED for f in core.tab(t2).filters), "imported filters flagged")

# history and missing file
tmp_csv = os.path.join(d, "temp.csv")
shutil.copy(ZERO, tmp_csv)
core.add_curve_from_file(t2, tmp_csv, pc.Role.COMPARE)
os.remove(tmp_csv)
h = core.history(s1)
ok(h[0].path == tmp_csv and not h[0].exists and h[1].exists, "history: missing file reported")
try:
    core.add_curve_from_file(t2, tmp_csv, pc.Role.COMPARE); ok(False, "missing file accepted")
except pc.CoreError.FileMissing:
    ok(True, "")
core.clear_history(s1, True)
ok(all(x.exists for x in core.history(s1)), "cleanup of missing entries")

# presets, duplicates, visibility, pinning
app_wide = pc.Visibility(mode=pc.VisibilityMode.ALL_EXCEPT, sessions=[])
p1 = core.create_preset_from_file(HARMAN, None, pc.PresetCategory.TARGET, False, app_wide)
p2 = core.create_preset_from_file(ZERO, "Zero (embedded)", pc.PresetCategory.MEASUREMENT, True,
                                  pc.Visibility(mode=pc.VisibilityMode.ONLY_LISTED, sessions=[s1]))
ok({p.id for p in core.presets(s1)} == {p1, p2}, "two presets visible in s1")
ok({p.id for p in core.presets(s2)} == {p1}, "preset restricted to s1 invisible in s2")
dup = core.find_duplicates_for_file(s2, HARMAN)
ok(len(dup) == 1 and dup[0].same_path, "duplicate by path")
dup = core.find_duplicates_for_file(s2, ZERO)
ok(len(dup) == 1 and not dup[0].same_path and not dup[0].visible_in_session, "duplicate by content")
core.grant_preset_to_session(p2, s2)
ok({p.id for p in core.presets(s2)} == {p1, p2}, "session added to the visibility")
core.set_preset_pinned(p2, s1, True)
ok(core.presets(s1)[0].id == p2 and core.presets(s1)[0].pinned, "pinned first")
ok(not core.presets(s2)[0].pinned or core.presets(s2)[0].id != p2, "pinning is per session")
t3 = core.create_tab(s2, None)
core.add_curve_from_preset(t3, p2, None)
core.add_curve_from_preset(t3, p1, None)
tv3 = core.tab(t3)
ok(tv3.measurement is not None and tv3.target is not None, "roles inferred from the category")
p3 = core.create_preset_from_curve(tab, pc.CurveRef.RESULT(), None, pc.PresetCategory.COMPENSATION, app_wide)
ok(next(p for p in core.presets(s1) if p.id == p3).embedded, "preset from a result")
core.update_preset_from_file(p2, ARIA)
ok(next(p for p in core.all_presets(s1) if p.id == p2).file_name == "Moondrop Aria.csv", "update from a file")
core.delete_preset(p3)
ok(all(p.id != p3 for p in core.all_presets(s1)), "deletion")

# inbox
it = core.send_to_inbox(tab, pc.CurveRef.RESULT(), "Aria corrected v1", s2)
ok([i.id for i in core.inbox(s2)] == [it], "inbox item received in s2")
s3 = core.create_session("Other", None)
ok(core.inbox(s3) == [], "invisible elsewhere")
core.add_curve_from_inbox(t3, it, pc.Role.TARGET)
ok(core.tab(t3).curves[-1].label == "Aria corrected v1", "taken as target")
ok(core.apply_inbox_filters(t3, it, True) == 11, "inbox item filters applied")

# deleted session -> zombie -> bindings moved over
core.delete_session(s2)
z = next(s for s in core.sessions(True) if s.id == s2)
ok(z.is_zombie and z.bindings == 2, f"zombie with its bindings ({z.bindings})")
ok(all(s.id != s2 for s in core.sessions(False)), "zombie missing from the normal list")
n = core.rebind_zombie(s2, s3)
ok(n == 2 and {p.id for p in core.presets(s3)} >= {p2}, "bindings moved over to s3")
ok(all(s.id != s2 for s in core.sessions(True)), "zombie erased after the move")
ok([i.id for i in core.inbox(s3)] == [it], "inbox item moved over")

# colors, offset
c = core.randomize_curve_color(tab, m)
ok(c.startswith("#") and len(c) == 7, "random color")
core.set_curve_color(tab, "result", "#123abc")
ok(core.tab(tab).result_color == "#123ABC", "manual result color")
core.set_curve_offset(tab, t, 5.0)
off = core.align_curve_to_target(tab, m)
ok(abs(off - core.tab(tab).curves[0].offset_db) < 1e-9, "alignment")

# compensation, free roles
s4 = core.active_session()
t4 = core.create_tab(s4, "Compensation")
cm = core.add_curve_from_file(t4, ARIA, pc.Role.MEASUREMENT)
cc = core.add_curve_from_file(t4, ARIA, pc.Role.COMPENSATION)
tv = core.tab(t4)
ok(tv.compensation == cc and next(c for c in tv.curves if c.id == cc).role == pc.Role.COMPENSATION, "compensation role")
pl = core.plot(t4, 300)
r = next(s for s in pl.series if s.key == "result")
ok(max(abs(v) for v in r.values) < 1e-6, "measurement − itself = 0, result shown without filters")
ok(any(s.kind == pc.SeriesKind.COMPENSATION for s in pl.series), "compensation series")
# a target used as the measurement, a measurement as the target
core.set_curve_role(t4, cc, pc.Role.COMPARE)
core.add_curve_from_file(t4, HARMAN, pc.Role.MEASUREMENT)
core.add_curve_from_file(t4, ZERO, pc.Role.TARGET)
core.add_curve_from_file(t4, ARIA, pc.Role.COMPENSATION)
core.set_tab_params(t4, 5, 1000, pc.Mode.IN_EAR)
core.run_autoeq(t4)
tv = core.tab(t4)
ok(len(tv.filters) == 5 and tv.compensation is not None, "autoeq with a target as measurement, and compensation")
ok("−" in next(s for s in core.plot(t4, 200).series if s.key == "result").label, "label of the compensated result")

# persistence
core.save()
core2 = pc.AppCore(d)
ok(len(core2.tab(tab).filters) == len(core.tab(tab).filters), "state reloaded")
ok(core2.active_session() == core.active_session(), "active session kept")
open(os.path.join(d, "state.json"), "w").write("{broken")
core3 = pc.AppCore(d)
ok(len(core3.sessions(False)) == 1 and glob.glob(os.path.join(d, "state.corrupt-*.json")), "corrupt file set aside")

shutil.rmtree(d)
print(f"OK: {checks} checks")
