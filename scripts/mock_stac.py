import json
from http.server import HTTPServer, BaseHTTPRequestHandler
import sys

class StacHandler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.endswith("/collections"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "collections": [{
                    "id": "mock-climate",
                    "title": "High-Resolution Local Climate",
                    "description": "Mock climate dataset containing air temperature, lat, lon, and time."
                }]
            }).encode())
        else:
            self.send_response(404)
            self.end_headers()

    def do_POST(self):
        if self.path.endswith("/search"):
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.end_headers()
            self.wfile.write(json.dumps({
                "features": [{
                    "assets": {
                        "data": {
                            "href": "climate_data.zarr",
                            "type": "application/vnd+zarr",
                            "title": "Local Climate Zarr",
                            "description": "Full N-dimensional Zarr group"
                        }
                    }
                }]
            }).encode())
        else:
            self.send_response(404)
            self.end_headers()

    def log_message(self, format, *args):
        pass # Suppress logging

if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8080
    print(f"Starting mock STAC server on port {port}")
    HTTPServer(("localhost", port), StacHandler).serve_forever()
