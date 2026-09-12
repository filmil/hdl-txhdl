# SPDX-License-Identifier: Apache-2.0
# Runs on the machine the board is attached to. Reads the serial port
# for a number of seconds and prints what comes, byte for byte as it
# arrives; once a whole line has come, types the reply, if any.
#
#   python3 serial.py PORT BAUD SECONDS [REPLY]
import sys
import time

import serial

port, baud, seconds = sys.argv[1], int(sys.argv[2]), float(sys.argv[3])
reply = sys.argv[4].encode() if len(sys.argv) > 4 else b""
s = serial.Serial(port, baud, timeout=0.1)
s.reset_input_buffer()
end = time.time() + seconds
seen = b""
replied = not reply
while time.time() < end:
    data = s.read(256)
    if data:
        sys.stdout.write(data.decode("latin1"))
        sys.stdout.flush()
        seen += data
    if not replied and b"\n" in seen:
        s.write(reply)
        s.flush()
        replied = True
print()
print(f"[serial] {len(seen)} bytes in {seconds:.0f} s")
