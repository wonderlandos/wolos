# send lines to the test VM serial console, print what comes back
import socket,sys,time,os
s=socket.socket(socket.AF_UNIX);s.connect(os.path.join(sys.argv.pop(1),'serial.sock'));s.settimeout(1)
out=b''
for line in sys.argv[1:]:
    s.send(line.encode()+b'\r'); end=time.time()+float(os.environ.get('WAIT','3'))
    while time.time()<end:
        try: out+=s.recv(65536)
        except socket.timeout: pass
print(out.decode(errors='replace'))
