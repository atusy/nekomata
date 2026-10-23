#!/usr/bin/env bash
# Run inside a disposable Debian ARM64 Vitro guest, never on the host.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
sudo apt-get -qq update
sudo -E apt-get -y -qq install linux-image-arm64 curl xz-utils git ca-certificates xserver-xorg lightdm openbox dbus-x11 xterm xdotool scrot x11-xserver-utils x11-utils xclip mesa-utils mesa-vulkan-drivers libgl1-mesa-dri fonts-noto-cjk fonts-noto-color-emoji
sudo mkdir -p /etc/lightdm/lightdm.conf.d
printf '[Seat:*]\nautologin-user=%s\nautologin-user-timeout=0\nuser-session=openbox\nautologin-session=openbox\n' "$(id -un)" | sudo tee /etc/lightdm/lightdm.conf.d/99-nekomata-lab.conf >/dev/null
guest_kernel=$(find /boot -maxdepth 1 -name 'vmlinuz-*-arm64' ! -name '*cloud*' -printf '%f\n' | sort -V | tail -1)
printf "GRUB_DEFAULT='Advanced options for Debian GNU/Linux>Debian GNU/Linux, with Linux %s'\n" "${guest_kernel#vmlinuz-}" | sudo tee /etc/default/grub.d/99-nekomata-desktop.cfg >/dev/null
sudo update-grub
mkdir -p "$HOME/.config/openbox"
printf 'xset s off\nxset -dpms\n' > "$HOME/.config/openbox/autostart"
sudo systemctl enable lightdm
sudo systemctl restart lightdm
if ! test -x "$HOME/.nix-profile/bin/nix"; then
  curl -fsSL https://nixos.org/nix/install -o /tmp/install-nix
  sh /tmp/install-nix --no-daemon --yes
fi
mkdir -p "$HOME/.config/nix"
printf 'experimental-features = nix-command flakes\n' > "$HOME/.config/nix/nix.conf"
