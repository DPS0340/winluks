#!/usr/bin/env python3
"""Compare disposable winluks RW copies against dm-crypt and original Linux plaintext.

The block oracle intentionally overwrites filesystem structures. Do not mount its copies.
All mappings are read-only and created only from regular files inside a Linux VM.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import secrets
import stat
import subprocess


def digest(path):
    with path.open('rb') as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def verify(manifest, writes):
    fixture = json.loads(manifest.read_text())
    report = json.loads((writes/'writes.json').read_text())
    source = manifest.parent/fixture['image']
    image = writes/'volume.img'
    plain = manifest.parent/fixture['plaintext']
    for path in (source, image, plain):
        if not stat.S_ISREG(path.lstat().st_mode):
            raise RuntimeError('Regular virtual-file fixtures required')
    assert fixture['name'] == report['fixture']
    assert digest(source) == fixture['image_sha256'] == report['source_image_sha256']
    assert digest(plain) == fixture['plaintext_sha256'] == report['source_plaintext_sha256']
    assert image.stat().st_size == source.stat().st_size
    header = report['header_bytes']
    assert 0 < header <= 64*1024*1024
    with source.open('rb') as a, image.open('rb') as b:
        original, current = a.read(header), b.read(header)
        assert original == current
        assert hashlib.sha256(current).hexdigest() == report['header_sha256']
    name = 'winluks-rw-verify-' + secrets.token_hex(6)
    subprocess.run(['cryptsetup','open','--readonly','--type','luks2','--key-slot',str(fixture['keyslot']),
                    '--key-file',str(manifest.parent/fixture['password_file']),str(image),name], check=True)
    try:
        expected_hash, actual_hash = hashlib.sha256(), hashlib.sha256()
        with plain.open('rb') as original, open('/dev/mapper/'+name,'rb',buffering=0) as decrypted:
            offset = 0
            while offset < report['plaintext_bytes']:
                length = min(1024*1024, report['plaintext_bytes']-offset)
                expected = bytearray(original.read(length))
                assert len(expected) == length
                for change in report['changes']:
                    begin = max(offset, change['offset'])
                    end = min(offset+length, change['offset']+change['length'])
                    if end > begin:
                        expected[begin-offset:end-offset] = bytes(
                            (17*i+change['seed']) % 256
                            for i in range(begin-change['offset'],end-change['offset']))
                actual = decrypted.read(length)
                assert actual == expected, f"Independent plaintext mismatch at {offset}"
                expected_hash.update(expected); actual_hash.update(actual)
                offset += length
            assert decrypted.read(1) == b''
        assert expected_hash.digest() == actual_hash.digest()
        return {'fixture':fixture['name'], 'passed':True, 'header_unchanged':True,
                'source_unchanged':True, 'bytes_compared':offset,
                'plaintext_sha256':actual_hash.hexdigest(), 'write_cases':len(report['changes'])}
    finally:
        subprocess.run(['cryptsetup','close',name], check=True)


def main():
    p = argparse.ArgumentParser()
    p.add_argument('--fixture-root', type=Path, required=True)
    p.add_argument('--writes-root', type=Path, required=True)
    p.add_argument('--results', type=Path, required=True)
    args = p.parse_args()
    if os.geteuid() != 0 or subprocess.run(['systemd-detect-virt','--vm','--quiet']).returncode:
        raise SystemExit('Run as root inside a disposable Linux VM')
    results = []
    reports = sorted(args.writes_root.rglob('writes.json'))
    if not reports: raise SystemExit('No block-write reports found')
    for report in reports:
        relative = report.relative_to(args.writes_root).parent
        result = verify(args.fixture_root/relative/'manifest.json', report.parent)
        results.append(result)
        print('PASS', result['fixture'], flush=True)
    args.results.write_text(json.dumps(results,indent=2)+'\n')


if __name__ == '__main__': main()
