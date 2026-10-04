#!/usr/bin/env python3
"""Test VMs for the installer, in the system libvirt (qemu:///system).

    vm.py create <name> --iso PATH [--usb] [--disk GB ...] [--secure-boot] [--media-first]
                 [--disk-file PATH ...] [--memory MB] [--sata] [--tpm]
    vm.py exec <name> <shell command>      run as root in the guest, print output
    vm.py ssh <name> <shell command>       run as root over SSH (installed systems)
    vm.py push <name> <local> <guest path> [--mode 755]   copy a file in
    vm.py wait <name> [seconds]            wait for the guest agent
    vm.py shot <name> <out.png>            screenshot the display
    vm.py destroy <name>                   stop and remove the VM and its disks

--tpm adds a TPM 2.0 (swtpm emulator, CRB), so the installer offers
encryption unlocked by the TPM (without it the VM has none); its state lives with the VM and goes with
`destroy`. Every VM is UEFI (OVMF), 8 GB RAM, 4 CPUs, virtio video without 3D (so
`virsh screenshot` can read it), a serial console and the guest agent
channel. Names are prefixed "atlasinst-", and disks live in build/vm/, so
nothing else in libvirt is ever touched. Commands run through the QEMU guest
agent, which AtlasOS enables.
"""

import argparse
import base64
import hashlib
import json
import pathlib
import subprocess
import sys
import time

URI = "qemu:///system"
PREFIX = "atlasinst-"
ROOT = pathlib.Path(__file__).resolve().parents[2]
VMDIR = ROOT / "build" / "vm"


def virsh(*args: str, check: bool = True) -> subprocess.CompletedProcess:
    return subprocess.run(["virsh", "-c", URI, *args], capture_output=True, text=True, check=check)


def domain(name: str) -> str:
    return name if name.startswith(PREFIX) else PREFIX + name


def agent(dom: str, command: str, **args) -> dict:
    r = virsh("qemu-agent-command", dom, json.dumps({"execute": command, "arguments": args}))
    return json.loads(r.stdout)["return"]


def create(a: argparse.Namespace) -> None:
    dom = domain(a.name)
    VMDIR.mkdir(parents=True, exist_ok=True)
    if virsh("dominfo", dom, check=False).returncode == 0:
        sys.exit(f"{dom} exists; destroy it first")
    sb = "yes" if a.secure_boot else "no"
    boot = (f"uefi,firmware.feature0.name=secure-boot,firmware.feature0.enabled={sb},"
            f"firmware.feature1.name=enrolled-keys,firmware.feature1.enabled={sb}")
    cmd = [
        "virt-install", "--connect", URI, "--name", dom,
        "--memory", str(a.memory), "--vcpus", "4", "--osinfo", "fedora-unknown",
        "--boot", boot,
        "--network", "network=default,model=virtio",
        "--graphics", "spice,listen=none", "--video", "virtio",
        "--channel", "unix,target.type=virtio,target.name=org.qemu.guest_agent.0",
        "--serial", "pty", "--console", "pty,target_type=serial",
        "--noautoconsole", "--import",
    ]
    # virt-install adds a TPM to every UEFI VM unless told not to
    cmd += ["--tpm", "backend.type=emulator,backend.version=2.0,model=tpm-crb" if a.tpm else "none"]
    bus = "sata" if a.sata else "virtio"
    # disks boot first unless --media-first (a disk with Windows on it would
    # boot instead of the installer)
    order = 2 if a.media_first else 1
    for i, gb in enumerate(a.disk):
        path = VMDIR / f"{dom}-disk{i}.qcow2"
        subprocess.run(["qemu-img", "create", "-q", "-f", "qcow2", str(path), f"{gb}G"], check=True)
        cmd += ["--disk", f"path={path},bus={bus},boot.order={order}"]
        order += 1
    for f in a.disk_file:
        cmd += ["--disk", f"path={pathlib.Path(f).resolve()},bus={bus},boot.order={order}"]
        order += 1
    # libvirt would chown the ISO to qemu and never give it back, so a rebuilt
    # ISO couldn't be written over it. It's world-readable, so qemu needs no chown.
    iso = pathlib.Path(a.iso).resolve()
    if a.media_first:
        order = 1
    if a.usb:
        cmd += ["--disk", f"path={iso},device=disk,bus=usb,readonly=on,boot.order={order},seclabel0.model=dac,seclabel0.relabel=no"]
    else:
        cmd += ["--disk", f"path={iso},device=cdrom,bus=sata,readonly=on,boot.order={order},seclabel0.model=dac,seclabel0.relabel=no"]
    subprocess.run(cmd, check=True)
    print(dom)


def wait(a: argparse.Namespace) -> None:
    dom = domain(a.name)
    end = time.time() + a.seconds
    while time.time() < end:
        try:
            agent(dom, "guest-ping")
            print("agent up")
            return
        except subprocess.CalledProcessError:
            time.sleep(3)
    sys.exit(f"no guest agent in {a.seconds} s")


def run(dom: str, command: str, timeout: int) -> tuple[int, str, str]:
    pid = agent(dom, "guest-exec", path="/bin/bash", arg=["-c", command], **{"capture-output": True})["pid"]
    end = time.time() + timeout
    while time.time() < end:
        st = agent(dom, "guest-exec-status", pid=pid)
        if st.get("exited"):
            dec = lambda k: base64.b64decode(st.get(k, "")).decode(errors="replace")
            return st.get("exitcode", -1), dec("out-data"), dec("err-data")
        time.sleep(1)
    raise TimeoutError(f"command still running after {timeout} s: {command}")


def exec_(a: argparse.Namespace) -> None:
    code, out, err = run(domain(a.name), a.command, a.timeout)
    sys.stdout.write(out)
    sys.stderr.write(err)
    sys.exit(code)


def push(a: argparse.Namespace) -> None:
    """Through the guest agent, in chunks that keep each virsh argument under
    Linux's 128 KiB limit for one argv string. Written beside the target and
    renamed over it, so a running binary can be replaced (no "text file
    busy")."""
    dom = domain(a.name)
    data = pathlib.Path(a.local).read_bytes()
    tmp = f"{a.guest}.push"
    h = agent(dom, "guest-file-open", path=tmp, mode="w")
    try:
        for i in range(0, len(data), 64 * 1024):
            agent(dom, "guest-file-write", handle=h, **{"buf-b64": base64.b64encode(data[i:i + 64 * 1024]).decode()})
    finally:
        agent(dom, "guest-file-close", handle=h)
    code, out, err = run(dom, f"chmod {a.mode} '{tmp}' && mv -f '{tmp}' '{a.guest}' && sha256sum '{a.guest}'", 30)
    want = hashlib.sha256(data).hexdigest()
    if code != 0 or not out.startswith(want):
        sys.exit(f"push failed: {out}{err}")
    print(f"{a.guest} ({len(data)} bytes)")


def ssh(a: argparse.Namespace) -> None:
    """For installed systems, where SELinux confines the guest agent (see test-access.sh)."""
    dom = domain(a.name)
    out = virsh("domifaddr", dom, "--source", "lease").stdout
    addrs = [f.split("/")[0] for line in out.splitlines() for f in line.split() if f.count(".") == 3 and "/" in f]
    if not addrs:
        sys.exit(f"{dom} has no DHCP lease yet")
    key = VMDIR / "id_test"
    r = subprocess.run(["ssh", "-i", str(key), "-o", "StrictHostKeyChecking=no", "-o", "UserKnownHostsFile=/dev/null",
                        "-o", "LogLevel=ERROR", "-o", "ConnectTimeout=10", "-o", "IdentitiesOnly=yes",
                        f"root@{addrs[-1]}", a.command])
    sys.exit(r.returncode)


def shot(a: argparse.Namespace) -> None:
    out = pathlib.Path(a.out).resolve()
    ppm = out.with_suffix(".ppm")
    virsh("screenshot", domain(a.name), str(ppm))
    subprocess.run(["magick", str(ppm), str(out)], check=True)
    ppm.unlink()
    print(out)


def destroy(a: argparse.Namespace) -> None:
    dom = domain(a.name)
    if virsh("dominfo", dom, check=False).returncode != 0:
        return
    disks = virsh("domblklist", "--details", dom).stdout.split("\n")
    virsh("destroy", dom, check=False)
    virsh("undefine", dom, "--nvram")
    for line in disks:
        f = line.split(maxsplit=3)  # the source path can hold spaces
        if len(f) == 4 and f[1] == "disk":
            p = pathlib.Path(f[3])
            if p.parent == VMDIR and p.name.startswith(dom):
                p.unlink(missing_ok=True)


def main() -> None:
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(required=True)
    c = sub.add_parser("create")
    c.add_argument("name")
    c.add_argument("--iso", required=True)
    c.add_argument("--usb", action="store_true", help="attach the ISO as a USB stick, not a CD")
    c.add_argument("--disk", type=int, action="append", default=[], help="a new blank disk of this many GB")
    c.add_argument("--disk-file", action="append", default=[], help="an existing disk image")
    c.add_argument("--secure-boot", action="store_true")
    c.add_argument("--media-first", action="store_true", help="boot the ISO before the disks")
    c.add_argument("--tpm", action="store_true", help="add a TPM 2.0 (swtpm emulator)")
    c.add_argument("--memory", type=int, default=8192)
    c.add_argument("--sata", action="store_true", help="target disks on SATA, not virtio (Windows has no virtio driver)")
    c.set_defaults(fn=create)
    w = sub.add_parser("wait")
    w.add_argument("name")
    w.add_argument("seconds", type=int, nargs="?", default=300)
    w.set_defaults(fn=wait)
    e = sub.add_parser("exec")
    e.add_argument("name")
    e.add_argument("command")
    e.add_argument("--timeout", type=int, default=120)
    e.set_defaults(fn=exec_)
    h = sub.add_parser("ssh")
    h.add_argument("name")
    h.add_argument("command")
    h.set_defaults(fn=ssh)
    u = sub.add_parser("push")
    u.add_argument("name")
    u.add_argument("local")
    u.add_argument("guest")
    u.add_argument("--mode", default="644")
    u.set_defaults(fn=push)

    s = sub.add_parser("shot")
    s.add_argument("name")
    s.add_argument("out")
    s.set_defaults(fn=shot)
    d = sub.add_parser("destroy")
    d.add_argument("name")
    d.set_defaults(fn=destroy)
    a = p.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
