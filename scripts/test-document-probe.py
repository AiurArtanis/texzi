"""External, byte-string oracle for the packaged in-memory component protocol.

This is the implementation's driver. Independent testers may replace it while
using the same executable/API. It proves no GUI/IO or performance acceptance.
"""
import argparse
import json
import pathlib
import queue
import random
import subprocess
import threading


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe", required=True)
    parser.add_argument("--trace", required=True)
    args = parser.parse_args()
    # CreateProcess does not normalize forward-slash relative executable paths.
    args.probe = str(pathlib.Path(args.probe).resolve(strict=True))
    trace = pathlib.Path(args.trace)
    if trace.exists():
        raise RuntimeError("preserve existing trace: " + str(trace))
    trace.parent.mkdir(parents=True, exist_ok=True)
    process = subprocess.Popen([args.probe], stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                               stderr=subprocess.PIPE, text=True, encoding="utf-8")
    replies, errors = queue.Queue(), []
    reader = threading.Thread(target=lambda: [replies.put(line) for line in process.stdout], daemon=True)
    error_reader = threading.Thread(target=lambda: errors.extend(process.stderr.readlines()), daemon=True)
    reader.start()
    error_reader.start()
    log = trace.open("x", encoding="utf-8")
    count = 0

    def ask(op, **fields):
        nonlocal count
        request = dict(op=op, **fields)
        process.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
        process.stdin.flush()
        try:
            response = json.loads(replies.get(timeout=30))
        except queue.Empty:
            raise RuntimeError("owned component worker gave no reply within 30s")
        count += 1
        log.write(json.dumps(dict(request=request, response=response), ensure_ascii=False) + "\n")
        log.flush()
        return response

    def ok(op, **fields):
        result = ask(op, **fields)
        assert result["ok"], result
        return result["result"]

    def state(doc="d"):
        return ok("state", doc=doc)

    def view(s, name):
        return next(v for v in s["views"] if v["name"] == name)

    def caret(at, affinity="after"):
        return dict(anchor=at, head=at, anchor_affinity=affinity, head_affinity=affinity)

    def replace(name, text, tick, kind="typing", doc="d"):
        s = state(doc)
        return ok("replace", doc=doc, view=name, epoch=view(s, name)["epoch"], text=text, tick=tick, kind=kind)

    def travel(op, name, doc="d"):
        s = state(doc)
        return ok(op, doc=doc, view=name, epoch=view(s, name)["epoch"])

    try:
        assert ok("hello")["protocol"] == 1
        ok("open", doc="d", text="")
        for name, mode in [("a", "source"), ("b", "rich")]:
            ok("open_view", doc="d", view=name, mode=mode, selection=[caret(0)])
        replace("a", "a", 0)
        saved = replace("a", "b", 750)
        assert saved["undo"] == 1 and saved["source"] == "ab"
        ok("save_begin", doc="d", save="save1", target="generated-memory-key")
        replace("a", "c", 751)
        after = replace("a", "d", 752)
        assert after["undo"] == 2
        ok("save_complete_simulated", doc="d", save="save1")
        after = travel("undo", "b")
        assert after["source"] == "ab" and after["token"] == saved["token"] and not after["dirty"]
        s = state()
        after = ok("mode", doc="d", view="b", epoch=view(s, "b")["epoch"], mode="source")
        assert (after["token"], after["revision"], after["undo"]) == (s["token"], s["revision"], s["undo"])
        assert travel("redo", "a")["source"] == "abcd"
        # Illegal selection_after must reject the body, view mapping AND history.
        before = state()
        bad = ask("submit", doc="d", view="a", epoch=view(before, "a")["epoch"],
                  base=before["revision"], edits=[dict(start=0, end=0, text="X")],
                  selection_revision=before["revision"] + 1, selection=[caret(999)], tick=800)
        assert not bad["ok"] and bad["error"] == "InvalidBoundary"
        assert state() == before
        bad = ask("replace", doc="d", view="a", epoch=0, text="BAD", tick=801)
        assert not bad["ok"] and bad["error"] == "StaleView"
        assert state() == before

        # Different algorithm: declarative prefix sums, not the Store's traversal.
        def map_marker(p, affinity, edits):
            delta_before = lambda a: sum(len(t) - (b - x) for x, b, t in edits if b <= a and x < b)
            for a, b, text in edits:
                if a <= p < b:
                    return a + delta_before(a) + (len(text) if affinity == "after" else 0)
            return (p + delta_before(p)
                    + sum(len(t) for a, b, t in edits if a == b and (a < p or a == p and affinity == "after")))

        rng = random.Random(20261117)
        ok("open", doc="r", text="abc中😀defghijk")
        for name in ["ra", "rb", "rc"]:
            ok("open_view", doc="r", view=name, mode="source", selection=[caret(0)])
        for case in range(128):
            before = state("r")
            raw = before["source"].encode("utf-8")
            boundaries = [len(before["source"][:i].encode("utf-8")) for i in range(len(before["source"]) + 1)]
            # Select four distinct boundaries: two disjoint nonempty patches.
            points = sorted(rng.sample(boundaries, 4))
            edits = [(points[0], points[1], rng.choice([b"X", "中".encode(), b""])),
                     (points[2], points[3], rng.choice([b"YY", "😀".encode(), b""]))]
            if all(raw[a:b] == t for a, b, t in edits):
                a, b, _ = edits[0]
                edits[0] = (a, b, b"ZZ")
            at, aff = rng.choice(boundaries), rng.choice(["before", "after"])
            ok("selection", doc="r", view="rc", epoch=view(before, "rc")["epoch"], selection=[caret(at, aff)])
            before = state("r")
            expected = raw
            for a, b, text in reversed(edits):
                expected = expected[:a] + text + expected[b:]
            response = ok("submit", doc="r", view="ra", epoch=view(before, "ra")["epoch"],
                          base=before["revision"], edits=[dict(start=a, end=b, text=t.decode()) for a, b, t in edits],
                          selection_revision=before["revision"] + 1, selection=[caret(0)], tick=case, kind="structural")
            assert response["source"].encode() == expected, case
            actual_marker = view(response, "rc")["selection"]["selection"][0]["head"]
            effective_edits = [(a, b, t) for a, b, t in edits if raw[a:b] != t]
            assert actual_marker == map_marker(at, aff, effective_edits), (case, at, aff, edits, actual_marker)
            shift, inverses = 0, []
            for a, b, text in edits:
                # NoOp patches need not appear in receipt.
                if raw[a:b] != text:
                    inverses.append(dict(start=a + shift, end=a + shift + len(text), text=raw[a:b].decode(), eols=[]))
                shift += len(text) - (b - a)
            assert response["receipt"]["inverse"] == inverses
            recovered = expected
            for d in reversed(inverses):
                recovered = recovered[:d["start"]] + d["text"].encode() + recovered[d["end"]:]
            assert recovered == raw
            undone = travel("undo", "rb", "r")
            assert undone["source"] == before["source"] and undone["token"] == before["token"]
            assert travel("redo", "rb", "r")["source"].encode() == expected
            travel("undo", "rb", "r")  # bounded independent cases, not a performance claim
        process.stdin.close()
        code = process.wait(timeout=10)
        assert code == 0, code
        print(json.dumps(dict(protocol=1, requests=count, random_cases=128, seed=20261117,
                              probe_exit=code, result="component checks passed")))
    finally:
        if process.poll() is None:
            process.kill()  # only this owned component worker, never the native application
            process.wait(timeout=10)
        reader.join(timeout=5)
        error_reader.join(timeout=5)
        if reader.is_alive() or error_reader.is_alive():
            raise RuntimeError("component stream reader did not finish")
        log.close()
        trace.with_suffix(".stderr.log").write_text("".join(errors), encoding="utf-8")


if __name__ == "__main__":
    main()
