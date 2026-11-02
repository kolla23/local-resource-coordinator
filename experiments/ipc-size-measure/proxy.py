#!/usr/bin/env python3
"""Usage: proxy.py LISTEN_SOCKET UPSTREAM_SOCKET LABEL_FILE RAW_OUT
Forwards each line unchanged; RAW_OUT gets its length, op and state counts, never payload bytes."""
import json
import os
import socket
import sys
import threading

RUNNING = {"starting", "running"}
ALL_STATES = {"queued", "starting", "running", "completed", "failed", "killed",
              "timeout", "expired", "oom", "cancelled", "skipped"}

out_lock = threading.Lock()


def count_states(value, counts):
    if isinstance(value, dict):
        state = value.get("state")
        if "id" in value and isinstance(state, str) and state in ALL_STATES:
            key = "q" if state == "queued" else "r" if state in RUNNING else "f"
            counts[key] += 1
        for item in value.values():
            count_states(item, counts)
    elif isinstance(value, list):
        for item in value:
            count_states(item, counts)


def states_of(line):
    counts = {"f": 0, "r": 0, "q": 0}
    try:
        count_states(json.loads(line), counts)
    except ValueError:
        return "unparsed"
    return "%d/%d/%d" % (counts["f"], counts["r"], counts["q"])


def op_of(line):
    try:
        value = json.loads(line)
    except ValueError:
        return "unparsed"
    if isinstance(value, dict) and isinstance(value.get("op"), str):
        return value["op"]
    return "unknown"


def record(out, conn, label, direction, op, size, states):
    with out_lock:
        out.write("%d\t%s\t%s\t%s\t%d\t%s\n" % (conn, label, direction, op, size, states))
        out.flush()


def pump(src, dst, out, conn, label, direction, last_op):
    reader = src.makefile("rb")
    try:
        while True:
            line = reader.readline()
            if not line:
                break
            body = line[:-1] if line.endswith(b"\n") else line
            if direction == "request":
                last_op[0] = op_of(body)
                record(out, conn, label, direction, last_op[0], len(body), "-")
            else:
                record(out, conn, label, direction, last_op[0], len(body), states_of(body))
            dst.sendall(line)
    except OSError:
        pass
    finally:
        try:
            dst.shutdown(socket.SHUT_WR)
        except OSError:
            pass


def serve(client, upstream_path, label_file, out, conn):
    with open(label_file, encoding="utf-8") as f:
        label = f.read().strip("\n")
    upstream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    try:
        upstream.connect(upstream_path)
    except OSError:
        client.close()
        return
    record(out, conn, label, "accept", "-", 0, "-")
    last_op = ["none"]
    up = threading.Thread(target=pump, args=(client, upstream, out, conn, label, "request", last_op))
    up.start()
    pump(upstream, client, out, conn, label, "reply", last_op)
    up.join()
    client.close()
    upstream.close()


def main():
    if len(sys.argv) != 5:
        sys.stderr.write(__doc__ + "\n")
        sys.exit(2)
    listen_path, upstream_path, label_file, raw_out = sys.argv[1:]
    server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    old = os.umask(0o077)
    server.bind(listen_path)
    os.umask(old)
    server.listen(64)
    conn = 0
    with open(raw_out, "a", encoding="utf-8") as out:
        while True:
            client, _ = server.accept()
            conn += 1
            threading.Thread(target=serve, args=(client, upstream_path, label_file, out, conn),
                             daemon=True).start()


if __name__ == "__main__":
    main()
