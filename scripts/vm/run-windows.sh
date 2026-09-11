#!/usr/bin/env bash
set -euo pipefail
# All disk arguments must refer to the private file-backed lab, never /dev devices.
if [[ $# -ne 3 ]]; then
  echo 'Usage: run-windows.sh LAB_DIRECTORY WINDOWS_ISO OVMF_CODE.secboot.fd' >&2
  exit 2
fi
lab=$(realpath "$1")
iso=$(realpath "$2")
firmware=$(realpath "$3")
for file in "$lab/windows.qcow2" "$lab/windows-vars.fd" "$lab/windows-seed.iso" "$iso" "$firmware"; do
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
  -drive "file=$iso,media=cdrom,readonly=on" \
  -drive "file=$lab/windows-seed.iso,media=cdrom,readonly=on" \
  -nic user,model=e1000e,hostfwd=tcp:127.0.0.1:22281-:22 \
  -device qemu-xhci -device usb-tablet -vga std -display none -vnc 127.0.0.1:18 \
  -boot order=c,once=d -qmp "unix:$lab/windows-qmp.sock,server=on,wait=off" \
  -pidfile "$lab/windows.pid" -daemonize
echo 'VM started. Complete the initial ISO boot prompt through VNC or QMP.'
