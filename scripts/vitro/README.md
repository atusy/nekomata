# Disposable desktop verification with Vitro and Nix

`host.sh` builds pinned Vitro 0.6.1 through Nix, obtains QEMU from the project's
nixpkgs input, and isolates its keys/config/disks under
`~/.local/share/nekomata-vitro-lab` (override `NEKOMATA_VITRO_LAB`). It does not
change the host desktop or use the normal felis daemon.

For a new lab, download Debian 13 ARM64's generic cloud qcow2 image from
<https://cloud.debian.org/images/cloud/trixie/latest/> and verify its published
checksum. The 2026-10-24 run used `debian-13-genericcloud-arm64.qcow2`, SHA256
`2e2da616418aab0474f4e582efaa0d90afe9bac63a0b5e8ecae0fb187a65cb0f`.
The `latest` URL is mutable; preserve the verified image for an exact rerun.

Run `bash scripts/vitro/host.sh keygen`. Create
`$NEKOMATA_VITRO_LAB/config/vitro/config.toml` (the default lab path above if the
variable is unset) using absolute paths:

```toml
[defaults]
cpus = 4
memory = "8G"
ssh_user = "dev"
resolution = "1440x900"
boot_timeout = "5m"
firmware = "/absolute/lab/tools/qemu/share/qemu/edk2-aarch64-code.fd"
[images.nekomata-linux]
[images.nekomata-linux.build]
kind = "cloud-image"
source = "/absolute/lab/debian.qcow2"
sha256 = "2e2da616418aab0474f4e582efaa0d90afe9bac63a0b5e8ecae0fb187a65cb0f"
disk_size = "40G"
```

Build/start the image with `host.sh build nekomata-linux` and
`host.sh run nekomata-linux`. Provision only the disposable guest:

```sh
bash scripts/vitro/host.sh exec --script scripts/vitro/provision-linux.sh nekomata-linux
bash scripts/vitro/host.sh restart nekomata-linux
bash scripts/vitro/host.sh exec --sync . nekomata-linux bash -lc \
  'git init; git add Cargo.toml Cargo.lock flake.nix flake.lock src tests vendor scripts share; bash scripts/vitro/check-linux.sh'
```

Sync from a clean checkout; Vitro excludes gitignored files, but untracked
worktrees inside the directory should be excluded from the source snapshot too.
The guest git index prevents Nix from copying build outputs into the flake.
The generic Debian kernel is required for virtio GPU support; reboot after
provisioning. `check-linux.sh` uses matching Nix Mesa/lavapipe libraries while
preserving the development shell's other runtime library paths.

Run the native keyboard/clipboard smoke test (default font size and lab resolution), comparison and GUI from the synced guest repository:

```sh
bash scripts/vitro/check-linux.sh cargo build --locked
bash scripts/vitro/check-linux.sh env DISPLAY=:0 XAUTHORITY="$HOME/.Xauthority" bash scripts/vitro/test-desktop.sh
bash scripts/vitro/check-linux.sh cargo test --release --locked benchmark_gpu_submission -- --ignored --nocapture --test-threads=1
mkdir -m 700 /tmp/nekomata-visual-lab
bash scripts/vitro/check-linux.sh env DISPLAY=:0 XAUTHORITY="$HOME/.Xauthority" \
  FELIS_SOCKET=/tmp/nekomata-visual-lab/felis.sock \
  NEKOMATA_RUNTIME_DIR=/tmp/nekomata-visual-lab/gui target/debug/nekomata
```

Inside that terminal run `python3 scripts/vitro/show-shared-features.py`, then
run it with `--palette` and `--reset`. It clears the screen, displays text,
decorations and Kitty pixels, and changes/resets palette slot 4 in place.
Capture from the host with `host.sh screenshot nekomata-linux --out FILE.png`.
If QEMU returns a blank inactive display, hover the VM and retry the capture.
Record the adapter: this lab uses llvmpipe/lavapipe, not native GPU hardware.
