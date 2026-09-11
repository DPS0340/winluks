#!/usr/bin/env bash
set -euo pipefail
# All disk arguments must refer to the private file-backed lab, never /dev devices.
if [[ $# -ne 2 && $# -ne 4 ]]; then
  echo 'Usage: run-windows.sh LAB_DIRECTORY OVMF_CODE.secboot.fd [--install WINDOWS_ISO]' >&2
  exit 2
fi
lab=$(realpath "$1")
firmware=$(realpath "$2")
media=()
boot=order=c
files=("$lab/windows.qcow2" "$lab/windows-vars.fd" "$firmware")
if [[ $# -eq 4 ]]; then
  [[ "$3" == --install ]] || { echo 'Expected --install' >&2; exit 2; }
  iso=$(realpath "$4")
  files+=("$lab/windows-seed.iso" "$iso")
  media=(-drive "file=$iso,media=cdrom,readonly=on" -drive "file=$lab/windows-seed.iso,media=cdrom,readonly=on")
  boot=order=c,once=d
fi
if [[ -f "$lab/windows.pid" ]] && kill -0 "$(cat "$lab/windows.pid")" 2>/dev/null; then
  echo 'VM is already running' >&2; exit 2
fi
for file in "${files[@]}"; do
  [[ -f "$file" && ! -b "$file" ]] || { echo 'Regular lab files required' >&2; exit 2; }
done
mkdir -p "$lab/tpm"
"${WINLUKS_SWTPM:-swtpm}" socket --tpm2 --tpmstate "dir=$lab/tpm" --ctrl "type=unixio,path=$lab/tpm.sock" --daemon
qemu-system-x86_64 -enable-kvm -machine q35,smm=on -cpu host -smp 4 -m 8192 \
  -global driver=cfi.pflash01,property=secure,value=on \
  -drive "if=pflash,format=raw,readonly=on,file=$firmware" \
  -drive "if=pflash,format=raw,file=$lab/windows-vars.fd" \
  -chardev "socket,id=chrtpm,path=$lab/tpm.sock" \
  -tpmdev emulator,id=tpm0,chardev=chrtpm -device tpm-crb,tpmdev=tpm0 \
  -drive "file=$lab/windows.qcow2,if=ide,format=qcow2,cache=none,aio=threads" \
  "${media[@]}" \
  -nic user,model=e1000e,hostfwd=tcp:127.0.0.1:22281-:22 \
  -device qemu-xhci -device usb-tablet -vga std -display none -vnc 127.0.0.1:18 \
  -boot "$boot" -qmp "unix:$lab/windows-qmp.sock,server=on,wait=off" \
  -pidfile "$lab/windows.pid" -daemonize
echo 'VM started. VNC is available on 127.0.0.1:5918.'
if [[ $# -eq 4 ]]; then
  echo 'Installation media is attached. Use --install only for a fresh disposable disk.'
fi
