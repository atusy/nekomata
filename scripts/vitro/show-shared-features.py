#!/usr/bin/env python3
"""Visual fixture for a private nekomata terminal in the Vitro desktop."""
import base64
import sys

if "--palette" in sys.argv:
    sys.stdout.write("\x1b]4;4;#ff00ff\x07")
    sys.stdout.flush()
    raise SystemExit
if "--reset" in sys.argv:
    sys.stdout.write("\x1b]104;4\x07")
    sys.stdout.flush()
    raise SystemExit

sys.stdout.write("\x1b[2J\x1b[HShared felis GPU renderer in Vitro\r\n\r\n")
sys.stdout.write("ASCII 0123456789 / 日本語 / e\u0301 / ⣿⣷⣶⣤⣀\r\n")
sys.stdout.write("\x1b[1mBold\x1b[0m  \x1b[3mItalic\x1b[0m  \x1b[4:3mCurly\x1b[0m  \x1b[4:4mDotted\x1b[0m  \x1b[4:5mDashed\x1b[0m\r\n")
sys.stdout.write("\x1b[44mPalette slot 4: this bar changes in place with OSC 4 / 104.       \x1b[0m\r\n\r\n")
sys.stdout.write("Kitty RGB image (red / green / blue / yellow):\r\n")
colors = [(240, 30, 30), (30, 220, 70), (30, 100, 240), (240, 210, 30)]
pixels = bytes(channel for _ in range(4) for color in colors for channel in color)
payload = base64.b64encode(pixels).decode("ascii")
sys.stdout.write(f"\x1b_Ga=T,f=24,s=4,v=4,i=77,c=24,r=6,z=1,q=2;{payload}\x1b\\")
sys.stdout.write("\r\n" * 7 + "Image and glyph atlases stay on the GPU.\r\n\x1b[4 q")
sys.stdout.flush()
