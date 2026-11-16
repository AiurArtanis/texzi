#!/usr/bin/env python3
"""Serve this design bundle locally. Usage: python tools/serve.py [port]."""
from functools import partial
from http.server import ThreadingHTTPServer, SimpleHTTPRequestHandler
from pathlib import Path
import sys

root = Path(__file__).resolve().parents[1]
port = int(sys.argv[1]) if len(sys.argv) > 1 else 8765
handler = partial(SimpleHTTPRequestHandler, directory=str(root))
print(f"texzi review: http://127.0.0.1:{port}/prototype/index.html")
print("Localhost only. Press Ctrl+C to stop.")
ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
