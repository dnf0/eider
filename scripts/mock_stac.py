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
                    "id": "cmip6-cesm2-historical",
                    "title": "CMIP6 CESM2 Historical Surface Temperature",
                    "description": "Near-surface air temperature from NCAR CESM2 historical runs (CMIP6). Monthly means, ~1° resolution, 1850–2014."
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
                            "href": "https://storage.googleapis.com/cmip6/CMIP6/CMIP/NCAR/CESM2/historical/r1i1p1f1/Amon/tas/gn/v20190308/tas",
                            "type": "application/vnd+zarr",
                            "title": "CMIP6 CESM2 Near-Surface Air Temperature",
                            "description": "Monthly mean near-surface air temperature (tas) from NCAR CESM2 historical simulation, 1850–2014"
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
