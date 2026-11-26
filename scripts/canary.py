#!/usr/bin/env python3
"""Live smoke test of the YouTube side, anonymous: search, home, resolve a stream, download its first bytes.
Needs the daemon built (./gradlew :daemon:installDist). Exits non-zero with the failing step. Used by the canary workflow."""
import json, os, queue, subprocess, sys, tempfile, threading, time, urllib.error, urllib.parse, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DAEMON = os.path.join(ROOT, "daemon", "build", "install", "daemon", "bin", "daemon")


def start(cfg):
    p = subprocess.Popen([DAEMON], env={**os.environ, "XDG_CONFIG_HOME": cfg}, stdout=subprocess.PIPE, stderr=open(os.path.join(cfg, "err.log"), "w"), text=True)
    q = queue.Queue()
    threading.Thread(target=lambda: q.put(p.stdout.readline()), daemon=True).start()
    try:
        hello = json.loads(q.get(timeout=90))
    except Exception as e:
        p.kill()
        sys.exit(f"daemon did not start: {e!r}")
    return p, f"http://127.0.0.1:{hello['port']}", hello["token"]


def call(base, token, path):
    req = urllib.request.Request(base + path, headers={"X-Metrolist-Token": token})
    return json.load(urllib.request.urlopen(req, timeout=60))


def check(name, fn, tries=2):
    for i in range(tries):
        try:
            out = fn()
            print(f"ok   {name}: {out}")
            return out
        except Exception as e:
            if i + 1 == tries:
                sys.exit(f"FAIL {name}: {e!r}")
            time.sleep(5)


def main():
    cfg = tempfile.mkdtemp(prefix="serafin-canary-")
    p, base, token = start(cfg)
    try:
        def search():
            r = call(base, token, "/youtube/search?q=" + urllib.parse.quote("daft punk get lucky"))
            assert r and r[0]["id"], "no results"
            return f"{len(r)} results, first {r[0]['title']!r}"
        check("search", search)

        def home():
            r = call(base, token, "/youtube/home")
            assert r and any(s["items"] for s in r), "no shelves"
            return f"{len(r)} shelves"
        check("home", home)

        def stream():
            s = call(base, token, "/stream/dQw4w9WgXcQ")
            assert s["url"], "no stream url"
            req = urllib.request.Request(s["url"], headers={**s.get("headers", {}), "Range": "bytes=0-65535"})
            with urllib.request.urlopen(req, timeout=60) as r:
                body = r.read()
                assert r.status == 206 and len(body) > 1000, f"HTTP {r.status}, {len(body)} bytes"
            return f"{s['client']} {s['mimeType']}, {len(body)} bytes"
        check("stream", stream)
    finally:
        p.kill()


main()
