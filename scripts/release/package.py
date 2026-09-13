#!/usr/bin/env python3
"""Package a tested CI artifact with matching application source and vendored dependencies."""
import argparse,hashlib,io,json,re,shutil,subprocess,tarfile,tempfile,zipfile
from pathlib import Path

def run(args,**kw):return subprocess.run(args,check=True,stdout=subprocess.PIPE,**kw).stdout

def sha(path):
    with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()

def main():
    p=argparse.ArgumentParser();p.add_argument('--artifact',type=Path,required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--version',required=True);p.add_argument('--build-commit',required=True);p.add_argument('--run-id',type=int,required=True);p.add_argument('--winspd-source',type=Path,required=True);a=p.parse_args()
    repo=Path(__file__).resolve().parents[2];head=run(['git','rev-parse','HEAD'],cwd=repo).decode().strip()
    assert not run(['git','status','--porcelain'],cwd=repo).strip(),'Commit reviewed sources before packaging'
    app_paths=['src','native/winspd_shim.c','build.rs','Cargo.toml','Cargo.lock','.github/workflows/ci.yml','.cargo','rust-toolchain','rust-toolchain.toml']
    run(['git','diff','--exit-code',a.build_commit,head,'--',*app_paths],cwd=repo)
    assert f'version = "{a.version}"' in (repo/'Cargo.toml').read_text()
    hashes={}
    for line in (a.artifact/'SHA256.txt').read_text(encoding='utf-8-sig').splitlines():
        digest,name=line.split(None,1);name=name.strip()
        assert re.fullmatch('[0-9a-fA-F]{64}',digest) and Path(name).name==name and name not in hashes
        hashes[name]=digest.lower();assert sha(a.artifact/name)==hashes[name]
    assert {'winluks2.exe','winspd-x64.dll','SOURCES.txt'}<=hashes.keys()
    source_line=f'Application source and build scripts: https://github.com/DPS0340/winluks/tree/{a.build_commit}'
    assert (a.artifact/'SOURCES.txt').read_text(encoding='utf-8-sig').splitlines()[0]==source_line
    # Bind the local files to the successful GitHub run and its exact uploaded ZIP.
    api='repos/DPS0340/winluks/actions'
    ci=json.loads(run(['gh','api',f'{api}/runs/{a.run_id}']))
    assert ci['head_sha']==a.build_commit and ci['conclusion']=='success' and ci['status']=='completed'
    assert ci['repository']['full_name']=='DPS0340/winluks' and ci['path']=='.github/workflows/ci.yml'
    artifacts=json.loads(run(['gh','api',f'{api}/runs/{a.run_id}/artifacts']))['artifacts']
    matches=[x for x in artifacts if x['name']=='winluks-windows-x64-experimental' and not x['expired']]
    assert len(matches)==1;artifact=matches[0]
    assert artifact['workflow_run']['head_sha']==a.build_commit
    uploaded=run(['gh','api',f"{api}/artifacts/{artifact['id']}/zip"])
    assert 'sha256:'+hashlib.sha256(uploaded).hexdigest()==artifact['digest']
    with zipfile.ZipFile(io.BytesIO(uploaded)) as z:
        for name in [*hashes,'SHA256.txt']:assert z.read(name)==(a.artifact/name).read_bytes(), f'CI artifact mismatch: {name}'
    # Unchanged DLL extracted from the SHA256-pinned WinSpd 1.0.20357 MSI in CI.
    assert hashes['winspd-x64.dll']=='35433b6e99c4b282a7ec07757f2206851f28dabf7bbcffd90bf60f317e865f7b'
    winspd='55c53bc454afbbba38bd1692f52beb77ed59142f'
    assert run(['git','rev-parse','HEAD'],cwd=a.winspd_source).decode().strip()==winspd
    a.output.mkdir(parents=True,exist_ok=False)
    with tempfile.TemporaryDirectory(prefix='winluks-package-') as tmp:
        stage=Path(tmp)
        source=stage/f'winluks-{a.version}-source';source.mkdir()
        archive=stage/'source.tar';archive.write_bytes(run(['git','archive',head],cwd=repo))
        with tarfile.open(archive) as tar:tar.extractall(source,filter='data')
        windows=stage/f'winluks-{a.version}-windows-x64';windows.mkdir()
        for name in ['winluks2.exe','winspd-x64.dll']:
            shutil.copy2(a.artifact/name,windows/name)
        for name in ['README.md','LICENSE','Cargo.lock']:
            shutil.copy2(source/name,windows/name)
        for name in ['docs','scripts','third_party','.github']:
            shutil.copytree(source/name,windows/name)
        shutil.copy2(source/'docs/START-HERE.md',windows/'START-HERE.md')
        (windows/'SOURCES.txt').write_text(f'Application CI source: https://github.com/DPS0340/winluks/tree/{a.build_commit}\nRelease source and evidence: https://github.com/DPS0340/winluks/tree/{head}\nCorresponding source with Rust dependencies and pinned WinSpd is shipped beside this archive.\n')
        winzip=a.output/f'winluks-{a.version}-windows-x64.zip'
        config=run(['cargo','vendor','--locked','--versioned-dirs',str(source/'vendor')],cwd=repo).decode()
        config=config.replace(str(source/'vendor'),'vendor');(source/'.cargo').mkdir(exist_ok=True);(source/'.cargo/config.toml').write_text(config)
        vendor_count=len(list((source/'vendor').iterdir()))
        # Carry actual dependency license/notice texts with the binary as well as
        # in corresponding source, preserving their package-relative locations.
        licenses=windows/'third_party/dependency-notices'
        notice_files={}
        for f in sorted((source/'vendor').rglob('*')):
            if f.is_file() and f.name.lower().startswith(('license','licence','copying','notice','copyright','readme','authors')):
                relative=f.relative_to(source/'vendor');target=licenses/relative;target.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(f,target)
                notice_files.setdefault(relative.parts[0],[]).append(Path(*relative.parts[1:]).as_posix())
        metadata=json.loads(run(['cargo','metadata','--locked','--all-features','--format-version','1'],cwd=repo))
        inventory=[{k:p.get(k) for k in ['name','version','license','license_file','source','repository']} for p in metadata['packages']]
        for item,package in zip(inventory,metadata['packages']):
            if item['license_file']:item['license_file']=str(Path(item['license_file']).relative_to(Path(package['manifest_path']).parent))
            item['included_notices']=notice_files.get(item['name']+'-'+item['version'],[])
            item['notice_coverage']='license-or-notice-file' if any(Path(n).name.lower().startswith(('license','licence','copying','notice','copyright')) for n in item['included_notices']) else ('readme-or-authors-only' if item['included_notices'] else ('application-license-at-package-root' if item['source'] is None else 'no-notice-file-in-published-crate'))
        (licenses/'inventory.json').write_text(json.dumps(inventory,indent=2)+'\n')
        (source/'third_party/winspd').mkdir()
        archive.write_bytes(run(['git','archive',winspd],cwd=a.winspd_source))
        with tarfile.open(archive) as tar:tar.extractall(source/'third_party/winspd',filter='data')
        (source/'BUILD-OFFLINE.md').write_text('Rust dependencies, including vendored OpenSSL source, are in vendor/. Cargo uses .cargo/config.toml.\nBuild the core with cargo build --offline --locked. Windows uses MSVC and --features winspd,vendored-openssl with the pinned WinSpd SDK described in .github/workflows/ci.yml.\nThe matching complete WinSpd source is in third_party/winspd/. Its release DLL is unchanged. Kernel-driver installers and Microsoft tools/media are obtained separately.\n')
        epoch=int(run(['git','show','-s','--format=%ct',head],cwd=repo))
        def normalize(info):
            info.uid=info.gid=0;info.uname=info.gname='';info.mtime=epoch;return info
        tarpath=a.output/f'winluks-{a.version}-source-with-dependencies.tar.gz'
        with tarfile.open(tarpath,'w:gz') as tar:tar.add(source,arcname=source.name,filter=normalize)
        (windows/'SHA256.txt').write_text(''.join(f'{sha(f)}  {f.relative_to(windows).as_posix()}\n' for f in sorted(windows.rglob('*')) if f.is_file()))
        # Published crates can carry Unix-epoch file times; ZIP starts at 1980.
        with zipfile.ZipFile(winzip,'w',zipfile.ZIP_DEFLATED,strict_timestamps=False) as z:
            for f in sorted(windows.rglob('*')):
                if f.is_file():z.write(f,f.relative_to(stage).as_posix())
    files=[winzip,tarpath]
    manifest={'version':a.version,'prerelease':True,'release_source_commit':head,'application_build_commit':a.build_commit,'ci_url':f'https://github.com/DPS0340/winluks/actions/runs/{a.run_id}','ci_artifact_id':artifact['id'],'ci_artifact_digest':artifact['digest'],'unchanged_application_paths':app_paths,'application_sha256':sha(a.artifact/'winluks2.exe'),'winspd_dll_sha256':sha(a.artifact/'winspd-x64.dll'),'winspd_source_commit':winspd,'rust_vendor_packages':vendor_count,'validation':f'https://github.com/DPS0340/winluks/blob/{head}/docs/VALIDATION.md','assets':{f.name:{'sha256':sha(f),'bytes':f.stat().st_size} for f in files},'review':'Two separate AI reviewers; not an external human audit or certification.','durability':'Orderly lock/dismount is the supported filesystem commit boundary. WinBtrfs application FlushFileBuffers durability is not guaranteed. QEMU SIGKILL tests do not power off the host storage device.'}
    manifest_path=a.output/'RELEASE-MANIFEST.json';manifest_path.write_text(json.dumps(manifest,indent=2)+'\n');files.append(manifest_path)
    (a.output/'SHA256SUMS.txt').write_text(''.join(f'{sha(f)}  {f.name}\n' for f in files))
    print(json.dumps(manifest,indent=2))
if __name__=='__main__':main()
