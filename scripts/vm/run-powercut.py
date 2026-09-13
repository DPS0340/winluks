#!/usr/bin/env python3
"""Reproduce hard-cut tests using fresh overlays of a prepared, powered-off lab guest.

Requires pexpect, the private lab layout documented in README.md, and an already running
Linux oracle VM. All mutations/cuts are scoped to the new trial directory, never the base.
"""
import argparse,base64,hashlib,json,os,re,select,shutil,signal,socket,subprocess,time
from pathlib import Path
import pexpect

def qmp(path,command,args=None):
    with socket.socket(socket.AF_UNIX) as sock:
        sock.settimeout(30);sock.connect(str(path));f=sock.makefile('rwb',buffering=0);json.loads(f.readline())
        for name,arguments in [('qmp_capabilities',{}),(command,args or {})]:
            f.write((json.dumps({'execute':name,'arguments':arguments})+'\n').encode())
            while True:
                r=json.loads(f.readline())
                if 'error' in r: raise RuntimeError(r['error'])
                if 'return' in r: break
        return r['return']
def main():
    p=argparse.ArgumentParser();p.add_argument('--lab',type=Path,required=True);p.add_argument('--repo',type=Path,required=True);p.add_argument('--trial',required=True);p.add_argument('--case',choices=['power-idle','power-flushed','power-stream','power-close','process-idle','process-flushed','process-stream','normal-close','disk-full','raw-block'],required=True);a=p.parse_args()
    assert re.fullmatch('[a-z0-9-]+',a.trial)
    lab=a.lab.resolve();repo=a.repo.resolve();base=lab/'windows-powercut-base';trial=lab/('windows-cut-'+a.trial);out=lab/'powercut'/a.trial
    assert not (base/'windows.pid').exists(),'Base must be powered off'
    trial.mkdir(mode=0o700);out.mkdir(mode=0o700)
    subprocess.run(['qemu-img','create','-q','-f','qcow2','-F','qcow2','-b',str(base/'windows.qcow2'),str(trial/'windows.qcow2')],check=True)
    shutil.copy2(base/'windows-vars.fd',trial/'windows-vars.fd');shutil.copytree(base/'tpm',trial/'tpm')
    env=dict(os.environ,WINLUKS_SWTPM=str(lab/'swtpm/bin/swtpm'))
    subprocess.run([str(repo/'scripts/vm/run-windows.sh'),str(trial),'/usr/share/edk2/x64/OVMF_CODE.secboot.4m.fd'],env=env,check=True)
    pid=int((trial/'windows.pid').read_text());common=['-o','BatchMode=yes','-o','ConnectTimeout=5','-o','UserKnownHostsFile='+str(lab/'vm/known_hosts'),'-i',str(lab/'vm/lab_ed25519')]
    def ssh(port):return ['ssh',*common,'-p',str(port),'lab@127.0.0.1']
    def ps_cmd(script):
        script="[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';"+script
        return ssh(22281)+['powershell.exe -NoProfile -ExecutionPolicy Bypass -EncodedCommand '+base64.b64encode(script.encode('utf-16le')).decode()]
    def ps(script):return subprocess.run(ps_cmd(script),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=60,check=True).stdout.decode('utf-8-sig')
    until=time.monotonic()+180
    while time.monotonic()<until:
        if subprocess.run(ssh(22281)+['cmd /c exit 0'],stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL,timeout=10).returncode==0:break
        time.sleep(2)
    else:raise RuntimeError('Guest did not boot')
    scp_windows=['scp','-q',*common,'-P','22281']
    for script_name in ['powercut-workload.ps1','powercut-raw.ps1']:
        subprocess.run(scp_windows+[str(repo/'scripts/windows'/script_name),'lab@127.0.0.1:C:/winluks-lab/scripts/'+script_name],check=True)
    binary_sha256=json.loads(ps(r"@{sha256=(Get-FileHash C:\winluks-lab\bin\winluks2.exe).Hash.ToLowerInvariant()} | ConvertTo-Json"))['sha256']
    key=(lab/'fixtures/btrfs-pbkdf2-512-sha256/password.key').read_text()
    remote=r"& C:\winluks-lab\bin\winluks2.exe open --image C:\winluks-lab\powercut\volume.img --keyslot 0 --filesystem btrfs --read-write; $code=$LASTEXITCODE; Write-Output ('APP_EXIT='+$code);exit $code"
    child=pexpect.spawn('ssh',['-tt',*common,'-p','22281','lab@127.0.0.1','powershell.exe -NoProfile -EncodedCommand '+base64.b64encode(remote.encode('utf-16le')).decode()],encoding='utf-8',timeout=90)
    log=(out/'console.txt').open('w');child.logfile_read=log;child.expect('Password:');child.send(key+'\r');child.expect('PUBLISHED_RW');print('PUBLISHED',a.trial,flush=True)
    info=json.loads(ps(r"$d=@(Get-Disk | Where-Object FriendlyName -match '^WinSpd winluks RW');if($d.Count -ne 1){throw 'Disk identity'};$v=@(Get-Volume | Where-Object FileSystem -eq 'BTRFS');if($v.Count -ne 1 -or !$v[0].DriveLetter){throw 'Volume identity'};@{disk=$d[0].Number;root=($v[0].DriveLetter+':\')} | ConvertTo-Json"))
    events=[];work=None;worklog=b''
    if not a.case.endswith('idle'):
        mode='raw' if a.case=='raw-block' else ('fill' if a.case=='disk-full' else ('stream' if a.case.endswith('stream') else 'flushed'))
        script=f"& C:\\winluks-lab\\scripts\\powercut-workload.ps1 -Root '{info['root']}' -DiskNumber {info['disk']} -Mode {mode}"
        if mode=='raw': script=f"& C:\\winluks-lab\\scripts\\powercut-raw.ps1 -Root '{info['root']}' -DiskNumber {info['disk']}"
        # select must observe the same unbuffered stream that readline consumes.
        # BufferedReader may prefetch the completion event and hide it from select.
        work=subprocess.Popen(ps_cmd(script),stdout=subprocess.PIPE,stderr=subprocess.STDOUT,bufsize=0)
        deadline=time.monotonic()+180
        while time.monotonic()<deadline:
            ready,_,_=select.select([work.stdout],[],[],1)
            if ready:
                line=work.stdout.readline();worklog+=line
                if not line:break
                if line.startswith(b'EVENT '):
                    event=json.loads(line[6:]);events.append(event)
                    if mode=='raw' and event.get('kind')=='raw-baseline':
                        assert len(events)==1, 'Duplicate or late baseline observation'
                        baseline=out/'raw-before.bin'
                        subprocess.run(scp_windows+['lab@127.0.0.1:C:/winluks-lab/powercut/raw-before.bin',str(baseline)],check=True,timeout=60)
                        assert baseline.is_file() and baseline.stat().st_size==event['bytes']
                        with baseline.open('rb') as f: assert hashlib.file_digest(f,'sha256').hexdigest()==event['sha256']
                        ps(r"New-Item -ItemType File C:\winluks-lab\powercut\raw-go | Out-Null")
                    if mode=='stream' and event.get('path'):break
                    if mode=='raw' and event.get('kind')=='workload-complete':break
        else:raise RuntimeError('Workload deadline')
        if mode not in ['stream','raw'] and a.case!='disk-full' and work.wait(timeout=10)!=0:raise RuntimeError('Workload failed')
        print('WORKLOAD',a.trial,len(events),flush=True)
    report={'binary_sha256':binary_sha256,'case':a.case,'trial':a.trial,'cut':'SIGKILL of verified trial QEMU PID','hard_cut':True,'events':events}
    if a.case.startswith('process-'):
        report['process_kill']=json.loads(ps(r"$p=@(Get-Process winluks2);if($p.Count -ne 1){throw 'Publisher identity'};Stop-Process -Id $p[0].Id -Force;Start-Sleep -Seconds 2;@{device_removed=(@(Get-Disk | Where-Object FriendlyName -match '^WinSpd winluks').Count -eq 0)} | ConvertTo-Json"))
    if a.case in ['normal-close','disk-full']:
        watcher=subprocess.Popen(ps_cmd(r"$p=Get-Process winluks2;$h=$p.Handle;[Console]::WriteLine('WATCHING');[Console]::Out.Flush();$p.WaitForExit();[Console]::WriteLine('APP_EXIT='+$p.ExitCode);[Console]::Out.Flush()"),stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
        if not select.select([watcher.stdout],[],[],30)[0] or b'WATCHING' not in watcher.stdout.readline():raise RuntimeError('No process exit observer')
    if a.case in ['normal-close','power-close','disk-full']:
        child.sendcontrol('c')
        if a.case!='power-close':
            child.expect(pexpect.EOF,timeout=90);child.wait();child.close()
            report['ssh_exit']=child.exitstatus
            observed,_=watcher.communicate(timeout=15)
            actual=re.search(rb'APP_EXIT=(\d+)',observed);assert actual, 'Process exit code unavailable'
            report['close_exit']=int(actual[1])
    # Freeze only the new QEMU process. SIGKILL skips guest shutdown and QEMU cleanup.
    cmd=Path(f'/proc/{pid}/cmdline').read_bytes();assert str(trial/'windows.qcow2').encode() in cmd
    if a.case=='raw-block':
        assert work.poll() is None, 'Raw workload exited before cut'
        assert events[-1]['kind']=='workload-complete', 'Raw workload did not reach held-lock boundary'
        report['raw_workload_alive_at_cut']=True
    os.kill(pid,signal.SIGKILL)
    for _ in range(50):
        try:
            if not Path(f'/proc/{pid}/cmdline').read_bytes():break
        except FileNotFoundError:break
        time.sleep(.1)
    if work:
        try:more,_=work.communicate(timeout=10);worklog+=more
        except subprocess.TimeoutExpired:work.kill();more,_=work.communicate();worklog+=more
    if child.isalive():
        try:child.expect(pexpect.EOF,timeout=10)
        except pexpect.TIMEOUT:child.close(force=True)
    log.close();text=re.sub(r'\x1b\[[0-?]*[ -/]*[@-~]','',(out/'console.txt').read_text());assert key not in text and key.encode() not in worklog
    events=[json.loads(line[6:]) for line in worklog.splitlines() if line.startswith(b'EVENT ')]
    if not a.case.endswith('idle'):
        if a.case=='raw-block':
            assert len([e for e in events if e.get('kind')=='block-flushed'])==3 and events[-1]['kind']=='workload-complete'
        elif a.case.endswith('stream'):
            assert any(e.get('path') for e in events), 'No observed write before cut'
        elif a.case=='disk-full':
            assert work.returncode != 0, 'Disk-full workload unexpectedly succeeded'
            assert any(e.get('kind')=='workload-error' and e.get('hresult')==-2147024784 for e in events), 'No confirmed ERROR_DISK_FULL'
        else:
            assert len([e for e in events if e.get('path')])==4, 'Missing file flush observations'
            assert events[-1]['kind']=='workload-complete', 'Incomplete workload'
    report['events']=events
    report['workload_exit']=work.returncode if work else None
    exit_match=re.search(r'APP_EXIT=(\d+)',text)
    if 'close_exit' not in report: report['close_exit']=int(exit_match[1]) if exit_match else None
    report['password_logged']=False;report['clean_close']='clean=true' in text
    close_failure=re.search(r'CLOSE_FAILED phase=(\d+) code=(\d+)',text)
    report['close_failure']={'phase':int(close_failure[1]),'code':int(close_failure[2])} if close_failure else None
    if a.case=='normal-close': assert report['close_exit']==0 and report['clean_close'], 'Orderly close failed'
    if a.case=='disk-full':
        assert report['close_exit']==1 and not report['clean_close'] and 'UNCLEAN_CLOSE' in text, 'Disk-full forced-RO failure was not reported'
        assert report['close_failure']=={'phase':0,'code':19}, 'Expected the pinned WinBtrfs forced-RO close rejection'
    (out/'events.json').write_text(json.dumps(events,indent=2)+'\n');(out/'trial.json').write_text(json.dumps(report,indent=2)+'\n');(out/'workload.txt').write_bytes(worklog)
    # Attach the UNBOOTED crash disk read-only to the Linux oracle guest. Windows never
    # gets a chance to repair NTFS or remount Btrfs before evidence is extracted.
    for attempt in range(15):
        try:
            qmp(lab/'vm/linux-qmp.sock','blockdev-add',{'driver':'qcow2','node-name':'cut-image','read-only':True,'file':{'driver':'file','filename':str(trial/'windows.qcow2'),'read-only':True}})
            break
        except RuntimeError as e:
            if 'lock' not in str(e) or attempt==14: raise
            time.sleep(1)
    qmp(lab/'vm/linux-qmp.sock','device_add',{'driver':'virtio-blk-pci','drive':'cut-image','id':'cut-device','serial':'winluks-cut','bus':'cut-port'})
    subprocess.run(ssh(22280)+[f'mkdir -p /home/lab/powercut/{a.trial}'],check=True)
    scp=['scp','-q',*common,'-P','22280']
    subprocess.run(scp+[str(out/'events.json'),f'lab@127.0.0.1:/home/lab/powercut/{a.trial}/events.json'],check=True)
    if a.case=='raw-block':subprocess.run(scp+[str(out/'raw-before.bin'),f'lab@127.0.0.1:/home/lab/powercut/{a.trial}/raw-before.bin'],check=True)
    subprocess.run(scp+[str(repo/'scripts/linux/verify-powercut.py'),'lab@127.0.0.1:/home/lab/verify-powercut.py'],check=True)
    time.sleep(2)
    required=f' --block-oracle --baseline /home/lab/powercut/{a.trial}/raw-before.bin' if a.case=='raw-block' else (' --require-records' if a.case=='normal-close' else '')
    result=subprocess.run(ssh(22280)+[f'sudo python3 /home/lab/verify-powercut.py --manifest /home/lab/fixtures/btrfs-pbkdf2-512-sha256/manifest.json --events /home/lab/powercut/{a.trial}/events.json --results /home/lab/powercut/{a.trial}/verified{required}'],stdout=subprocess.PIPE,stderr=subprocess.STDOUT,timeout=90)
    (out/'linux.txt').write_bytes(result.stdout)
    subprocess.run(scp+[f'lab@127.0.0.1:/home/lab/powercut/{a.trial}/verified/result.json',str(out/'linux.json')],check=True)
    qmp(lab/'vm/linux-qmp.sock','device_del',{'id':'cut-device'});time.sleep(2);qmp(lab/'vm/linux-qmp.sock','blockdev-del',{'node-name':'cut-image'})
    print('VERIFIED',a.trial,'exit',result.returncode,flush=True)
    if result.returncode:raise SystemExit(result.returncode)
if __name__=='__main__':main()
