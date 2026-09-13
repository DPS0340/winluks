#!/usr/bin/env python3
"""Read an unbooted, read-only Windows crash disk and verify its LUKS fixture independently."""
import argparse,hashlib,json,os,secrets,shutil,subprocess,tempfile
from pathlib import Path

def run(args,check=True): return subprocess.run(args,check=check,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
def sha(p):
    with Path(p).open('rb') as f: return hashlib.file_digest(f,'sha256').hexdigest()
def main():
    p=argparse.ArgumentParser();p.add_argument('--manifest',type=Path,required=True);p.add_argument('--events',type=Path,required=True);p.add_argument('--results',type=Path,required=True);p.add_argument('--require-records',action='store_true');p.add_argument('--block-oracle',action='store_true');p.add_argument('--baseline',type=Path);a=p.parse_args()
    if os.geteuid()!=0 or run(['systemd-detect-virt','--vm','--quiet'],False).returncode: raise SystemExit('Root in disposable VM required')
    devices=json.loads(run(['lsblk','--tree','-J','-b','-o','PATH,SIZE,RO,FSTYPE,SERIAL']).stdout)['blockdevices']
    disks=[d for d in devices if d.get('serial')=='winluks-cut' and d['ro']]
    assert len(disks)==1
    parts=[d for d in disks[0]['children'] if d.get('fstype') in ('ntfs','BitLocker') and d['ro']]
    part=max(parts,key=lambda d:d['size'])
    assert part['size']>32*1024**3
    a.results.mkdir(exist_ok=False,parents=True)
    image=a.results/'volume.img'
    with tempfile.TemporaryDirectory(prefix='winluks-ntfs-ro-') as tmp, tempfile.TemporaryDirectory(prefix='winluks-outer-ro-') as outer:
        # Our evaluation guest automatically enables used-space BitLocker with a clear
        # key. Dislocker supports that state; no credentials or writable mapping are used.
        bitlocker=part['fstype']=='BitLocker'
        if bitlocker: run(['dislocker','-r','-c','-V',part['path'],'--',outer])
        try:
            ntfs=str(Path(outer)/'dislocker-file') if bitlocker else part['path']
            run(['ntfs-3g','-o','ro',ntfs,tmp])
            try:
                shutil.copyfile(Path(tmp)/'winluks-lab/powercut/volume.img',image)
            finally: run(['umount',tmp])
        finally:
            if bitlocker: run(['umount',outer])
    m=json.loads(a.manifest.read_text()); source=a.manifest.parent/m['image']
    assert sha(source)==m['image_sha256']; assert image.stat().st_size==source.stat().st_size
    meta=json.loads(run(['cryptsetup','luksDump','--dump-json-metadata',str(source)]).stdout); offset=int(meta['segments']['0']['offset'])
    with source.open('rb') as x,image.open('rb') as y: assert x.read(offset)==y.read(offset)
    events=json.loads(a.events.read_text()); records=[e for e in events if e.get('path')]
    if a.require_records: assert len(records)==4 and events[-1]['kind']=='workload-complete', 'Missing required flush records'
    name='winluks-cut-'+secrets.token_hex(5); dev='/dev/mapper/'+name
    run(['cryptsetup','open','--readonly','--type','luks2','--key-slot',str(m['keyslot']),'--key-file',str(a.manifest.parent/m['password_file']),str(image),name])
    result={'header_unchanged':True,'canonical_unchanged':True,'image_size_unchanged':True,'image_sha256':sha(image),'acknowledged_records':len(records)}
    try:
        if a.block_oracle:
            changes=[e for e in events if e.get('kind')=='block-flushed']
            assert len(changes)==3 and events[-1]['kind']=='workload-complete'
            total=int(meta['segments']['0'].get('size','0')) if meta['segments']['0'].get('size')!='dynamic' else image.stat().st_size-offset
            plaintext=a.manifest.parent/m['plaintext']
            assert plaintext.stat().st_size==m['plaintext_bytes']==total, 'Plaintext reference length mismatch'
            assert sha(plaintext)==m['plaintext_sha256'], 'Plaintext reference hash mismatch'
            assert [(c['offset'],c['length'],c['seed']) for c in changes]==[(0,512,83),(131072,1048576,112),(total-512,512,141)], 'Unexpected raw write observations'
            baseline_events=[e for e in events if e.get('kind')=='raw-baseline']
            assert len(baseline_events)==1 and events[0]==baseline_events[0], 'Missing prewrite baseline observation'
            baseline=baseline_events[0];before=a.baseline
            assert before and before.is_file() and not before.is_symlink(), 'Regular prewrite baseline required'
            assert before.stat().st_size==baseline['bytes']==total
            assert sha(before)==baseline['sha256'], 'Prewrite baseline hash mismatch'
            with open(dev,'rb') as actual,before.open('rb') as original:
                position=0
                while position<total:
                    expected=bytearray(original.read(min(1024*1024,total-position)))
                    assert expected
                    for c in changes:
                        start=max(position,c['offset']);end=min(position+len(expected),c['offset']+c['length'])
                        if end>start: expected[start-position:end-position]=bytes((17*i+c['seed'])%256 for i in range(start-c['offset'],end-c['offset']))
                    assert actual.read(len(expected))==expected, f'Block durability mismatch at {position}'
                    position+=len(expected)
                assert actual.read(1)==b''
            result.update(block_oracle=True,acknowledged_block_writes=3,bytes_compared=position,required_checks_passed=True,prewrite_baseline_sha256=baseline['sha256'],prewrite_baseline_authenticated=True,canonical_plaintext_reference_verified=True,baseline_source='Windows raw read after volume lock/dismount and before the tested writes')
            (a.results/'result.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result));return
        fsck=run(['btrfs','check','--readonly',dev],False);(a.results/'btrfs-check.txt').write_bytes(fsck.stdout)
        result['btrfs_check_exit']=fsck.returncode
        with open(dev,'rb') as f:
            f.seek(65536);primary=f.read(4096);f.seek(67108864);secondary=f.read(4096)
        result['mirror_generations']=[int.from_bytes(b[72:80],'little') for b in (primary,secondary)]
        result['mirrors_agree']=primary[32:48]==secondary[32:48] and primary[56:]==secondary[56:]
        with tempfile.TemporaryDirectory(prefix='winluks-cut-ro-') as tmp:
            mounted=run(['mount','-t','btrfs','-o','ro,nologreplay',dev,tmp],False)
            result['read_only_mount_exit']=mounted.returncode
            if mounted.returncode==0:
                try:
                    root=Path(tmp)
                    result['canonical_files_match']=all((root/f).is_file() and sha(root/f)==info['sha256'] for f,info in m['files'].items())
                    result['records']=[]
                    for e in records:
                        target=root/e['path']; match=target.is_file() and target.stat().st_size==e['size'] and sha(target)==e['sha256']
                        result['records'].append({'path':e['path'],'matches':match,'present':target.is_file()})
                    result['all_acknowledged_records_match']=all(r['matches'] for r in result['records'])
                finally: run(['umount',tmp])
            else: result['mount_error']=mounted.stdout.decode(errors='replace')[-1024:]
    finally: run(['cryptsetup','close',name])
    result['required_records']=a.require_records
    result['required_checks_passed']=result.get('canonical_files_match',False) and (not a.require_records or result.get('all_acknowledged_records_match',False)) and result['btrfs_check_exit']==0
    (a.results/'result.json').write_text(json.dumps(result,indent=2)+'\n'); print(json.dumps(result))
    if not result['required_checks_passed']: raise SystemExit(1)
if __name__=='__main__':main()
