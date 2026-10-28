# Record the test VM's screen to a video: QMP screendump at a fixed rate, then ffmpeg.
# Usage: python3 record.py <test dir with qmp.sock> <out.mp4> [seconds=60] [fps=10]
# Frames go to <test dir>/frames; the VM keeps running. Another client can drive it (qmp.py keys).
import json, os, shutil, socket, subprocess, sys, time

T, out = sys.argv[1], sys.argv[2]
secs = float(sys.argv[3]) if len(sys.argv) > 3 else 60
fps = int(sys.argv[4]) if len(sys.argv) > 4 else 10
d = os.path.join(T, "frames")
shutil.rmtree(d, ignore_errors=True)
os.makedirs(d)
s = socket.socket(socket.AF_UNIX)
s.connect(os.path.join(T, "qmp.sock"))
f = s.makefile("rw")
f.readline()


def cmd(c, a=None):
    f.write(json.dumps({"execute": c, **({"arguments": a} if a else {})}) + "\n")
    f.flush()
    while True:
        r = json.loads(f.readline())
        if "return" in r or "error" in r:
            return r


cmd("qmp_capabilities")
n, start = 0, time.monotonic()
while time.monotonic() - start < secs:
    due = start + n / fps
    time.sleep(max(0, due - time.monotonic()))
    cmd("screendump", {"filename": os.path.join(d, f"{n:06d}.ppm"), "format": "ppm"})
    n += 1
# Frames are taken on schedule; a late frame repeats the time slot, so the video keeps real time.
subprocess.run(
    ["ffmpeg", "-loglevel", "error", "-y", "-framerate", str(fps), "-i", os.path.join(d, "%06d.ppm"),
     "-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2", "-c:v", "libx264", "-pix_fmt", "yuv420p", "-crf", "23", out],
    check=True,
)
shutil.rmtree(d)
print(f"{out}: {n} frames, {n / fps:.0f} s")
