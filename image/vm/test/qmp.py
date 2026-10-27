# QMP helper for the test VM: shot <png> | keys <text> | key <qcode[-qcode]>...
import socket,sys,json,os,time
T=sys.argv.pop(1)   # test dir (qmp.sock)
s=socket.socket(socket.AF_UNIX);s.connect(T+'/qmp.sock');f=s.makefile('rw');f.readline()
def cmd(c,a=None):
    f.write(json.dumps({'execute':c,**({'arguments':a} if a else {})})+'\n');f.flush()
    while True:
        r=json.loads(f.readline())
        if 'return' in r or 'error' in r: return r
cmd('qmp_capabilities')
M={' ':'spc','\n':'ret','=':'equal',',':'comma','.':'dot','/':'slash','-':'minus',';':'semicolon',"'":'apostrophe','[':'bracket_left',']':'bracket_right','\t':'tab','`':'grave_accent','\\':'backslash'}
S={'"':'apostrophe','_':'minus',':':'semicolon','|':'backslash','>':'dot','<':'comma','*':'8','$':'4','&':'7','(':'9',')':'0','~':'grave_accent','!':'1','#':'3','{':'bracket_left','}':'bracket_right','?':'slash','+':'equal','@':'2','%':'5','^':'6'}
def send(keys): cmd('send-key',{'keys':[{'type':'qcode','data':k} for k in keys],'hold-time':30}); time.sleep(.09)
a=sys.argv[1]
if a=='shot':
    cmd('screendump',{'filename':T+'/q.ppm'})
    from PIL import Image; Image.open(T+'/q.ppm').save(sys.argv[2])
elif a=='keys':
    for ch in sys.argv[2]:
        if ch in M: send([M[ch]])
        elif ch in S: send(['shift',S[ch]])
        elif ch.isupper(): send(['shift',ch.lower()])
        else: send([ch])
elif a=='key':
    for k in sys.argv[2:]: send(k.split('-')); time.sleep(.2)
