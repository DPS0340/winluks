#!/usr/bin/env python3
"""Generate disposable virtual-file fixtures inside a Linux VM, never on real disks.

Credentials are synthetic random UTF-8 bytes stored only in mode-0600 local files.
The production CLI does not accept key files. This harness does, solely for tests.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import secrets
import shutil
import subprocess
import tempfile


def run(args, **kw):
    return subprocess.run(args, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, **kw).stdout


def sha(path):
    with open(path, 'rb', buffering=0) as f:
        return hashlib.file_digest(f, 'sha256').hexdigest()


def fixture(root, fs, kdf, bits, hash_name, slot_bits=None, alternate_slot=False):
    name = f'{fs}-{kdf}-{bits}-{hash_name}' + (f'-slot{slot_bits}' if slot_bits else '')
    if alternate_slot:
        name += '-alternate-slot'
    dest = root / name
    dest.mkdir(mode=0o700)
    image, key, plain = dest/'volume.img', dest/'password.key', dest/'plaintext.raw'
    key.write_bytes(('합성시험-' + secrets.token_hex(24)).encode())
    key.chmod(0o600)
    with image.open('xb') as f:
        f.truncate(256 * 1024 * 1024)
    args = ['cryptsetup', 'luksFormat', '--batch-mode', '--type', 'luks2', '--cipher',
            'aes-xts-plain64', '--key-size', str(bits), '--sector-size', '512',
            '--hash', hash_name, '--pbkdf', kdf, '--key-file', str(key)]
    if kdf == 'pbkdf2':
        args += ['--pbkdf-force-iterations', '1000']
    else:
        args += ['--pbkdf-force-iterations', '4', '--pbkdf-memory', '32768', '--pbkdf-parallel', '1']
    if slot_bits:
        args += ['--keyslot-key-size', str(slot_bits), '--keyslot-cipher', 'aes-xts-plain64']
    run(args + [str(image)])
    selected_slot = 0
    if alternate_slot:
        old_key = dest/'old-password.key'
        key.rename(old_key)
        key.write_bytes(('다른슬롯-' + secrets.token_hex(24)).encode())
        key.chmod(0o600)
        run(['cryptsetup','luksAddKey','--batch-mode','--key-slot','1','--key-file',str(old_key),
             '--new-keyfile',str(key),'--pbkdf','pbkdf2','--pbkdf-force-iterations','1000',
             '--hash','sha512' if hash_name=='sha256' else 'sha256',str(image)])
        selected_slot = 1
    name_map = 'winluks-fixture-' + secrets.token_hex(6)
    mapped = Path('/dev/mapper') / name_map
    mounted = False
    opened = False
    with tempfile.TemporaryDirectory(prefix='winluks-mount-') as tmp:
        mount = Path(tmp)
        try:
            run(['cryptsetup', 'open', '--type', 'luks2', '--key-file', str(key), str(image), name_map])
            opened = True
            if fs == 'ext4':
                features = 'none,has_journal,extent,filetype,metadata_csum,64bit,flex_bg,dir_index,ext_attr,sparse_super,large_file,huge_file,dir_nlink,extra_isize'
                mkfs = ['mkfs.ext4', '-q', '-b', '4096', '-I', '256', '-O', features,
                        '-E', 'lazy_itable_init=0,lazy_journal_init=0', str(mapped)]
            else:
                mkfs = ['mkfs.btrfs', '-q', '-f', '-s', '4096', '-n', '16384',
                        '-m', 'single', '-d', 'single', '-O', 'extref,skinny-metadata,no-holes',
                        '-R', '^free-space-tree', str(mapped)]
            run(mkfs)
            run(['mount', '-t', fs, str(mapped), str(mount)])
            mounted = True
            (mount/'unicode-한글').mkdir()
            (mount/'hello.txt').write_text('winluks fixture\n', encoding='utf-8')
            (mount/'unicode-한글'/'파일.txt').write_text('읽기 전용 파일 검증\n', encoding='utf-8')
            (mount/'empty').touch()
            (mount/'pattern.bin').write_bytes(bytes(range(256))*16384)
            with (mount/'sparse.bin').open('wb') as f:
                f.write(b'begin'); f.seek(32*1024*1024-3); f.write(b'end')
            if fs == 'btrfs':
                run(['btrfs', 'subvolume', 'create', str(mount/'subvolume')])
                (mount/'subvolume'/'nested.txt').write_text('subvolume fixture\n')
            files = {str(p.relative_to(mount)): {'sha256':sha(p), 'size':p.stat().st_size}
                     for p in mount.rglob('*') if p.is_file()}
            run(['umount', str(mount)]); mounted = False
            run(['cryptsetup', 'close', name_map]); opened = False
            before = sha(image)
            run(['cryptsetup', 'open', '--readonly', '--type', 'luks2', '--key-file', str(key), str(image), name_map]); opened = True
            check = ['e2fsck', '-fn', str(mapped)] if fs == 'ext4' else ['btrfs', 'check', '--readonly', str(mapped)]
            run(check)
            with mapped.open('rb', buffering=0) as src, plain.open('xb') as dst:
                shutil.copyfileobj(src, dst, 1024*1024)
            opts = 'ro,noload' if fs == 'ext4' else 'ro,nologreplay'
            run(['mount', '-t', fs, '-o', opts, str(mapped), str(mount)]); mounted = True
            for relative, info in files.items():
                assert sha(mount/relative) == info['sha256']
            run(['umount', str(mount)]); mounted = False
            run(['cryptsetup', 'close', name_map]); opened = False
            assert sha(image) == before
            metadata = json.loads(run(['cryptsetup', 'luksDump', '--dump-json-metadata', str(image)]))
            # Metadata includes only synthetic salts/digests. Keep it out of shared logs.
            (dest/'metadata.local.json').write_text(json.dumps(metadata, indent=2))
            selected = metadata['keyslots'][str(selected_slot)]
            manifest = dict(name=name,filesystem=fs,kdf=selected['kdf']['type'],key_bits=bits,
                            hash=selected['af']['hash'],keyslot=selected_slot,
                            keyslot_key_bits=selected['area']['key_size']*8,
                            digest_hash=metadata['digests']['0']['hash'],
                            different_slot_password=alternate_slot,
                            image='volume.img',password_file='password.key',plaintext='plaintext.raw',
                            image_sha256=before,plaintext_sha256=sha(plain),plaintext_bytes=plain.stat().st_size,
                            files=files,kernel=platform.release(),cryptsetup=run(['cryptsetup','--version']).decode().strip(),
                            creation_args=[('<synthetic-password-file>' if x==str(key) else x) for x in args]+['<virtual-image>'],
                            mkfs_args=mkfs[:-1]+['<virtual-mapping>'],oracle_mount_options=opts)
            (dest/'manifest.json').write_text(json.dumps(manifest, ensure_ascii=False, indent=2)+'\n')
            print(f'{name}: image, plaintext and file oracle verified', flush=True)
        finally:
            if mounted:
                run(['umount', str(mount)])
            if opened:
                run(['cryptsetup', 'close', name_map])


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--output', type=Path, required=True)
    ap.add_argument('--matrix', action='store_true')
    ap.add_argument('--cross-key-sizes', action='store_true')
    ap.add_argument('--alternate-slot', action='store_true', help='different password and hash in slot 1; original slot 0 remains')
    args = ap.parse_args()
    if os.geteuid() != 0:
        ap.error('run as root inside a disposable Linux VM')
    if subprocess.run(['systemd-detect-virt','--vm','--quiet']).returncode:
        ap.error('fixture mounts must run inside a VM')
    root = args.output.resolve()
    root.mkdir(parents=True, mode=0o700, exist_ok=True)
    combos = [('pbkdf2',512,'sha256')]
    if args.matrix:
        combos = [(k,b,h) for k in ['pbkdf2','argon2i','argon2id'] for b in [256,512]
                  for h in ['sha256','sha512']]
    for fs in ['ext4','btrfs']:
        if args.cross_key_sizes:
            for bits,slot in [(256,512),(512,256)]:
                fixture(root,fs,'pbkdf2',bits,'sha256',slot,args.alternate_slot)
            continue
        for kdf,bits,hash_name in combos:
            fixture(root,fs,kdf,bits,hash_name,alternate_slot=args.alternate_slot)


if __name__ == '__main__':
    main()
