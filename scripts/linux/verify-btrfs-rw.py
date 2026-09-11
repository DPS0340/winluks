#!/usr/bin/env python3
"""Validate a detached Windows-written synthetic image and optionally write a return file."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess
import tempfile


def run(args):
    return subprocess.run(args, check=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT).stdout


def sha(path):
    with path.open('rb') as f: return hashlib.file_digest(f,'sha256').hexdigest()


def main():
    p=argparse.ArgumentParser()
    p.add_argument('--manifest',type=Path,required=True)
    p.add_argument('--image',type=Path,required=True)
    p.add_argument('--expected',type=Path,required=True)
    p.add_argument('--results',type=Path,required=True)
    p.add_argument('--write-return-file',action='store_true')
    args=p.parse_args()
    if os.geteuid()!=0 or subprocess.run(['systemd-detect-virt','--vm','--quiet']).returncode:
        raise SystemExit('Run as root inside a disposable Linux VM')
    m=json.loads(args.manifest.read_text(encoding='utf-8-sig'))
    expected=json.loads(args.expected.read_text(encoding='utf-8-sig'))
    source=args.manifest.parent/m['image'];key=args.manifest.parent/m['password_file']
    assert m['filesystem']=='btrfs' and expected['fixture']==m['name']
    assert args.image.resolve()!=source.resolve()
    for path in (source,args.image): assert stat.S_ISREG(path.lstat().st_mode)
    assert sha(source)==m['image_sha256']
    assert args.image.stat().st_size==source.stat().st_size
    metadata=json.loads(run(['cryptsetup','luksDump','--dump-json-metadata',str(source)]))
    offset=int(metadata['segments']['0']['offset'])
    with source.open('rb') as a,args.image.open('rb') as b: assert a.read(offset)==b.read(offset)
    assert sha(args.image)!=m['image_sha256']
    args.results.mkdir(exist_ok=False)
    name='winluks-files-rw-'+secrets.token_hex(6);device='/dev/mapper/'+name
    def open_volume(ro):
        run(['cryptsetup','open','--type','luks2']+(['--readonly'] if ro else [])+
            ['--key-slot',str(m['keyslot']),'--key-file',str(key),str(args.image),name])
    def check():
        text=run(['btrfs','check','--readonly',device])
        (args.results/'btrfs-check.txt').write_bytes(text)
    open_volume(True)
    try:
        check()
        with tempfile.TemporaryDirectory(prefix='winluks-rw-mount-') as tmp:
            root=Path(tmp);run(['mount','-t','btrfs','-o','ro,nologreplay',device,str(root)])
            try:
                for relative,info in expected['files'].items():
                    path=root/relative
                    assert path.stat().st_size==info['size'] and sha(path)==info['sha256'], relative
                for relative in expected['absent']: assert not (root/relative).exists(), relative
                payload=bytearray((i*17+31)%256 for i in range(600123))
                payload[511:1536]=bytes((i*17+91)%256 for i in range(1025))
                payload.extend(bytes((i*17+149)%256 for i in range(256123)))
                assert (root/'rw-tests/done/payload.bin').read_bytes()==payload[:730111]
                assert (root/'rw-tests/done/한글-파일.txt').read_text()=='Windows RW 검증'
                assert (root/'hello-renamed.txt').read_text()=='winluks fixture\nRW append\n'
                sparse=root/'rw-tests/done/sparse.bin'
                assert sparse.stat().st_blocks*512<sparse.stat().st_size
            finally: run(['umount',str(root)])
    finally: run(['cryptsetup','close',name])
    if args.write_return_file:
        open_volume(False)
        try:
            with tempfile.TemporaryDirectory(prefix='winluks-rw-return-') as tmp:
                root=Path(tmp);run(['mount','-t','btrfs','-o','noatime,space_cache=v1',device,str(root)])
                try:
                    path=root/'rw-tests/done/linux-return.txt'
                    with path.open('xb') as f:
                        f.write('Linux에서 기록하고 Windows에서 다시 읽기\n'.encode());f.flush();os.fsync(f.fileno())
                    expected['files']['rw-tests/done/linux-return.txt']={'sha256':sha(path),'size':path.stat().st_size}
                finally: run(['umount',str(root)])
            check()
        finally: run(['cryptsetup','close',name])
    (args.results/'expected.json').write_text(json.dumps(expected,ensure_ascii=False,indent=2)+'\n')
    result={'passed':True,'filesystem':'btrfs','header_unchanged':True,'canonical_source_unchanged':True,
            'btrfs_check_readonly_passed':True,'windows_file_hashes_match':True,
            'payload_matches_independent_pattern':True,'sparse_allocation_verified':True,
            'linux_return_file_written':args.write_return_file,'file_count':len(expected['files']),
            'image_sha256':sha(args.image)}
    (args.results/'linux-rw.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps(result))


if __name__=='__main__':main()
