import json, sys, threading, time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

LOG = "spike-log.jsonl"
HEADS = ["Origin", "Sec-Fetch-Site", "Access-Control-Request-Method",
         "Access-Control-Request-Headers", "Access-Control-Request-Private-Network"]
lock = threading.Lock()


def log(obj):
    with lock, open(LOG, "a") as f:
        f.write(json.dumps(obj) + "\n")


class H(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, *a):
        pass

    def cors(self):
        origin = self.headers.get("Origin")
        if origin:
            self.send_header("Access-Control-Allow-Origin", origin)
        self.send_header("Vary", "Origin")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, DELETE")
        self.send_header("Access-Control-Allow-Headers", "content-type, accept")
        self.send_header("Access-Control-Expose-Headers", "Retry-After")
        if self.headers.get("Access-Control-Request-Private-Network"):
            self.send_header("Access-Control-Allow-Private-Network", "true")

    def record(self):
        log({"method": self.command, "path": self.path,
             "headers": {k: self.headers.get(k) for k in HEADS}})

    def body(self):
        n = int(self.headers.get("Content-Length") or 0)
        return self.rfile.read(n) if n else b""

    def do_OPTIONS(self):
        self.record()
        self.send_response(204)
        self.cors()
        self.send_header("Content-Length", "0")
        self.end_headers()

    def handle_any(self):
        self.record()
        data = self.body()
        if self.path == "/report" and self.command == "POST":
            log({"report": json.loads(data or b"{}")})
            self.send_response(200)
            self.cors()
            self.send_header("Content-Length", "2")
            self.end_headers()
            self.wfile.write(b"{}")
            self.wfile.flush()
            threading.Thread(target=self.server.shutdown).start()
            return
        if self.path == "/stream":
            self.send_response(200)
            self.cors()
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Cache-Control", "no-cache")
            self.send_header("Connection", "close")
            self.end_headers()
            try:
                for i in range(5):
                    self.wfile.write(f"data: {i}\n\n".encode())
                    self.wfile.flush()
                    time.sleep(0.3)
            except OSError:
                pass
            self.close_connection = True
            return
        body = b'{"ok":true}'
        self.send_response(200)
        self.cors()
        self.send_header("Retry-After", "7")
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    do_GET = do_POST = do_DELETE = handle_any


if __name__ == "__main__":
    srv = ThreadingHTTPServer(("127.0.0.1", 47613), H)
    print("listening", flush=True)
    srv.serve_forever()
