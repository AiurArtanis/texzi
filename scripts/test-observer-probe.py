"""Implementation driver for C08; independent clients can replace this script."""
import argparse
import hashlib
import json
import os
import pathlib
import queue
import subprocess
import threading


class Worker:
    def __init__(self, exe, directory):
        self.directory = pathlib.Path(directory)
        self.directory.mkdir()  # never overwrite a previous round
        self.p = subprocess.Popen([exe], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        self.q = queue.Queue()
        self.count = self.rejects = 0
        self.raw = (self.directory / "stdout.bin").open("xb")
        self.stderr = (self.directory / "stderr.bin").open("xb")
        self.trace = (self.directory / "trace.jsonl").open("x", encoding="utf-8")
        identity = dict(probe=str(pathlib.Path(exe).resolve()), probe_sha256=hashlib.sha256(pathlib.Path(exe).read_bytes()).hexdigest(),
                        pid=self.p.pid, commit=os.environ.get("GITHUB_SHA"), run=os.environ.get("GITHUB_RUN_ID"), attempt=os.environ.get("GITHUB_RUN_ATTEMPT"))
        (self.directory / "identity.json").write_text(json.dumps(identity, indent=2) + "\n", encoding="utf-8")
        def read():
            for line in self.p.stdout:
                self.raw.write(line)
                self.raw.flush()
                self.q.put(line)
        def read_error():
            for line in self.p.stderr:
                self.stderr.write(line)
                self.stderr.flush()
        self.reader = threading.Thread(target=read, daemon=True)
        self.error_reader = threading.Thread(target=read_error, daemon=True)
        self.reader.start()
        self.error_reader.start()

    def ask(self, op, **fields):
        request = dict(op=op, **fields)
        self.p.stdin.write((json.dumps(request, ensure_ascii=False) + "\n").encode())
        self.p.stdin.flush()
        response = json.loads(self.q.get(timeout=30))
        self.count += 1
        self.rejects += not response["ok"]
        self.trace.write(json.dumps(dict(request=request, response=response), ensure_ascii=False) + "\n")
        self.trace.flush()
        return response

    def ok(self, op, **fields):
        response = self.ask(op, **fields)
        assert response["ok"], response
        return response["result"]

    def fail(self, op, prefix, **fields):
        response = self.ask(op, **fields)
        assert not response["ok"] and response["error"].startswith(prefix), response

    def setup(self, journal=64, queue_max=64):
        self.ok("obs_config", journal=journal, queue_max=queue_max, subscriptions_max=32)
        self.ok("open", doc="d", text="abc")
        for name, at in [("a", 0), ("b", 3)]:
            self.ok("open_view", doc="d", view=name, selection=[dict(anchor=at, head=at)])

    def edit(self, text):
        s = self.ok("state", doc="d")
        at = len(s["source"].encode())
        return self.ok("service_submit", doc="d", base=s["revision"], edits=[dict(start=at, end=at, text=text)])

    def nav(self, view, at):
        s = self.ok("state", doc="d")
        epoch = next(v["epoch"] for v in s["views"] if v["name"] == view)
        return self.ok("selection", doc="d", view=view, epoch=epoch, selection=[dict(anchor=at, head=at)])

    def observe(self, name, views=None, capacity=8):
        return self.ok("obs_observe", doc="d", views=views or ["a"], capacity=capacity, snapshot=name, handle=name)

    def cache(self, name, snapshot, install=True):
        self.ok("obs_cache_prepare", cache=name, snapshot=snapshot, buffer_frames=128)
        if install:
            return self.ok("obs_cache_install", cache=name)
        return self.cs(name)

    def cs(self, name):
        return self.ok("obs_cache_state", cache=name)

    def poll(self, handle, name):
        return self.ok("obs_poll", handle=handle, poll=name)

    def frames(self, cache, poll, batch):
        for i in range(len(batch["frames"])):
            self.ok("obs_cache_frame", cache=cache, poll=poll, frame=i)
        return self.cs(cache)

    def close(self):
        self.p.stdin.close()
        killed = False
        try:
            code = self.p.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.p.kill()  # only this owned protocol worker on a failing test
            code = self.p.wait(timeout=10)
            killed = True
        self.reader.join(timeout=5)
        self.error_reader.join(timeout=5)
        self.raw.close()
        self.stderr.close()
        self.trace.close()
        result = dict(requests=self.count, expected_or_observed_rejects=self.rejects,
                      worker_exit=code, cleanup_killed=killed)
        (self.directory / "exit.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
        assert code == 0 and not killed and not self.reader.is_alive() and not self.error_reader.is_alive(), result
        return result


def initial_buffer(w):
    w.setup()
    snap = w.observe("s")
    w.cache("c", "s", install=False)
    w.edit("!")
    b = w.poll("s", "p")
    assert b["cursor"]["seq"] == snap["cursor"]["seq"] + 1
    assert [e["type"] for e in b["events"]] == ["Content", "Global", "View"]
    before = w.cs("c")
    w.frames("c", "p", b)
    assert w.cs("c")["state"] is None
    w.fail("obs_cache_resume_cursor", "NeedsObserve", cache="c")
    c = w.ok("obs_cache_install", cache="c")
    assert c["state"]["global"]["document"]["source"] == "abc!"
    assert c["cursor"]["seq"] == before["cursor"]["seq"] + 1
    frozen = w.ok("obs_snapshot", snapshot="s")
    assert frozen == dict(cursor=snap["cursor"], subscription=snap["subscription"], state=snap["state"])
    new = w.observe("after")
    assert new["cursor"] == c["cursor"] and w.poll("after", "empty")["type"] == "Pending"


def save_resume(w):
    w.setup()
    w.ok("save_begin", doc="d", save="save", target="memory-fixture")
    snap = w.observe("s")
    w.cache("c", "s")
    w.ok("obs_unsubscribe", handle="s")
    w.ok("save_complete_simulated", doc="d", save="save")
    w.ok("obs_resume", doc="d", views=["a"], cache="c", handle="resumed", capacity=8)
    w.ok("fence", doc="d")
    for i in range(6):  # five save-state publications followed by a live history fence
        b = w.poll("resumed", "p" + str(i))
        assert b["type"] == "Batch"
        w.frames("c", "p" + str(i), b)
    c = w.cs("c")
    g = c["state"]["global"]
    assert g["document"]["revision"] == snap["state"]["global"]["document"]["revision"] == 0
    assert c["cursor"]["seq"] == snap["cursor"]["seq"] + 6
    assert g["save"]["active"] is None and g["save"]["last_terminal"]["result"] == "SucceededByCaller"
    assert g["baseline"]["kind"] == "SavedByCaller" and not g["dirty"]
    assert g["history"]["last_fence"] == "Explicit"


def duplicates_gap_partial(w):
    w.setup()
    w.observe("s")
    baseline = w.cache("c", "s")
    w.edit("X")
    b = w.poll("s", "p")
    for i in [0, 1, 1]:
        w.ok("obs_cache_frame", cache="c", poll="p", frame=i)
    half = w.cs("c")
    assert half["state"] == baseline["state"] and half["cursor"] == baseline["cursor"] and half["partial"]
    for i in range(2, len(b["frames"])):
        w.ok("obs_cache_frame", cache="c", poll="p", frame=i)
    complete = w.cs("c")
    w.frames("c", "p", b)
    assert w.cs("c") == complete
    w.fail("obs_cache_frame", "Protocol", cache="c", poll="p", frame=len(b["frames"]) - 1, parts=999)
    failed = w.cs("c")
    assert failed["stale"] and failed["state"] == complete["state"] and failed["cursor"] == complete["cursor"]
    w.observe("s2")
    old = w.cache("c2", "s2")
    w.edit("1")
    w.edit("2")
    w.poll("s2", "first")
    w.poll("s2", "second")
    w.fail("obs_cache_frame", "Protocol", cache="c2", poll="second", frame=0)
    assert w.cs("c2")["cursor"] == old["cursor"]
    w.observe("s3")
    old = w.cache("c3", "s3")
    w.edit("3")
    b = w.poll("s3", "third")
    w.ok("obs_cache_frame", cache="c3", poll="third", frame=0)
    w.fail("obs_cache_frame", "Protocol", cache="c3", poll="third", frame=len(b["frames"]) - 1)
    assert w.cs("c3")["state"] == old["state"]


def projection_and_identity(w):
    w.setup()
    w.nav("b", 1)
    w.observe("s")
    baseline = w.cache("c", "s")
    w.nav("b", 2)
    b = w.poll("s", "empty")
    assert b["events"] == [] and len(b["frames"]) == 2
    c = w.frames("c", "empty", b)
    assert c["state"] == baseline["state"] and c["cursor"]["seq"] == baseline["cursor"]["seq"] + 1
    w.observe("s2", views=["a", "b"])
    baseline = w.cache("c2", "s2")
    response = w.ok("obs_cache_frame", cache="c2", poll="empty", frame=0, seq=9999)
    assert response["outcome"] == "IgnoredOldHandle" and w.cs("c2") == baseline
    w.nav("b", 1)
    w.poll("s2", "active")
    w.fail("obs_cache_frame", "Protocol", cache="c2", poll="active", frame=0, seq=0, schema=99)
    assert w.cs("c2")["stale"]


def twice_lagged(w):
    w.setup(journal=2, queue_max=2)
    snap = w.observe("s", capacity=1)
    w.cache("c", "s", install=False)
    w.nav("a", 1)
    w.nav("a", 2)
    lag = w.poll("s", "lag")
    assert lag["type"] == "Lagged"
    w.ok("obs_cache_control", cache="c", poll="lag")
    w.fail("obs_cache_install", "NeedsObserve", cache="c")
    w.fail("obs_poll", "StaleSubscription", handle="s", poll="terminal")
    assert w.cs("c")["cursor"] == snap["cursor"]
    w.nav("a", 3)
    w.fail("obs_resume_raw", "CursorTooOld", doc="d", views=["a"], cursor=snap["cursor"], handle="too-old", capacity=2)
    second = w.observe("s2", capacity=1)
    w.cache("c2", "s2", install=False)
    w.ok("save_begin", doc="d", save="save", target="memory-fixture")
    w.ok("save_complete_simulated", doc="d", save="save")
    lag = w.poll("s2", "lag2")
    assert lag["type"] == "Lagged"
    w.ok("obs_cache_control", cache="c2", poll="lag2")
    w.fail("obs_cache_install", "NeedsObserve", cache="c2")
    assert w.cs("c2")["cursor"] == second["cursor"]
    third = w.observe("s3", capacity=1)
    assert third["cursor"]["seq"] == lag["info"]["head"]
    c = w.cache("c3", "s3")
    assert c["state"]["global"]["baseline"]["kind"] == "SavedByCaller"
    assert c["state"]["global"]["save"]["last_terminal"]["result"] == "SucceededByCaller"
    w.nav("a", 0)
    b = w.poll("s3", "next")
    c = w.frames("c3", "next", b)
    assert w.ok("obs_cache_control", cache="c3", poll="lag2")["outcome"] == "IgnoredOldHandle"
    assert w.cs("c3") == c


def zero_log_and_close(w):
    w.setup(journal=0, queue_max=8)
    first = w.observe("s")
    w.cache("c", "s")
    w.ok("obs_unsubscribe", handle="s")
    w.ok("obs_resume", doc="d", views=["a"], cache="c", handle="r", capacity=8)
    assert w.poll("r", "pending")["type"] == "Pending"
    w.edit("X")
    b = w.poll("r", "edit")
    c = w.frames("c", "edit", b)
    w.edit("Y")
    w.ok("obs_unsubscribe", handle="r")
    w.fail("obs_resume_raw", "CursorTooOld", doc="d", views=["a"], cursor=c["cursor"], handle="old", capacity=8)
    w.observe("s2")
    w.cache("c2", "s2")
    current = w.ok("state", doc="d")
    epoch = next(v["epoch"] for v in current["views"] if v["name"] == "a")
    w.ok("close_view", doc="d", view="a", epoch=epoch)
    b = w.poll("s2", "closed-view")
    c = w.frames("c2", "closed-view", b)
    assert c["state"]["views"][0]["status"] == "Closed"
    w.ok("obs_close_document", doc="d")
    gone = w.poll("s2", "gone")
    assert gone["type"] == "DocGone"
    w.ok("obs_cache_control", cache="c2", poll="gone")
    assert w.cs("c2")["stale"]
    w.fail("obs_resume_raw", "DocGone", doc="d", views=["a"], cursor=first["cursor"], handle="gone-r", capacity=8)
    return first["cursor"]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--probe", required=True)
    parser.add_argument("--trace-dir", required=True)
    args = parser.parse_args()
    root = pathlib.Path(args.trace_dir)
    root.mkdir(parents=True, exist_ok=False)
    summaries = []
    saved_cursor = None
    for index, case in enumerate([initial_buffer, save_resume, duplicates_gap_partial, projection_and_identity, twice_lagged, zero_log_and_close]):
        w = Worker(args.probe, root / f"round-{index + 1:02}-{case.__name__}")
        try:
            result = case(w)
            if result:
                saved_cursor = result
        finally:
            summaries.append(dict(case=case.__name__, **w.close()))
    w = Worker(args.probe, root / "round-07-restart-epoch")
    try:
        w.setup()
        fresh = w.observe("s")
        assert fresh["cursor"]["store"] != saved_cursor["store"]
        w.fail("obs_resume_raw", "WrongStore", doc="d", views=["a"], cursor=saved_cursor, handle="old-store", capacity=8)
    finally:
        summaries.append(dict(case="restart_epoch", **w.close()))
    result = dict(rounds=summaries, requests=sum(s["requests"] for s in summaries),
                  rejects=sum(s["expected_or_observed_rejects"] for s in summaries), all_worker_exit_zero=True)
    (root / "summary.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result))


if __name__ == "__main__":
    main()
