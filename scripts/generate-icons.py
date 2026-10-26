#!/usr/bin/env python3
"""Rasterize the source SVG with resvg; run via the Nix development shell."""
from pathlib import Path
import struct
import subprocess
import tempfile
import xml.etree.ElementTree as ET

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "share/icons"
SVG = "http://www.w3.org/2000/svg"
ET.register_namespace("", SVG)
source = ET.parse(ICONS / "nekomata.svg").getroot()


def render(svg, size, destination):
    with tempfile.NamedTemporaryFile(suffix=".svg") as temporary:
        ET.ElementTree(svg).write(temporary.name)
        subprocess.run(
            ["resvg", "--width", str(size), "--height", str(size), temporary.name, str(destination)],
            check=True,
        )


# Keep the original silhouette, with a warm background for desktop/Dock contrast.
app = ET.fromstring(ET.tostring(source))
app.insert(0, ET.Element(f"{{{SVG}}}rect", {
    "x": "64", "y": "64", "width": "1126", "height": "1126",
    "rx": "250", "fill": "#ece6da",
}))
render(app, 256, ICONS / "nekomata.png")

# Crop the original whitespace for legibility in the small palette button.
button = ET.fromstring(ET.tostring(source))
button.set("viewBox", "305 300 750 750")
for path in button.findall(f"{{{SVG}}}path"):
    path.set("fill", "#ffffff")
render(button, 128, ICONS / "palette-button.png")

# ICNS supports PNG payloads at these sizes, on every build host.
chunks = []
with tempfile.TemporaryDirectory() as directory:
    for kind, size in [(b"ic07", 128), (b"ic08", 256), (b"ic09", 512), (b"ic10", 1024)]:
        png = Path(directory) / f"{size}.png"
        render(app, size, png)
        data = png.read_bytes()
        chunks.append(kind + struct.pack(">I", len(data) + 8) + data)
payload = b"".join(chunks)
(ICONS / "nekomata.icns").write_bytes(b"icns" + struct.pack(">I", len(payload) + 8) + payload)
