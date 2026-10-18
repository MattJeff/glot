#!/usr/bin/env bash
# One-time hardening of a fresh Ubuntu 24.04 VPS (Hetzner CX/CAX). Idempotent: safe to re-run.
#   scp deploy/bootstrap.sh root@IP: && ssh root@IP bash bootstrap.sh
# Login key: root's authorized_keys (the key picked when ordering the server), or GLOT_SSH_PUBKEY="ssh-ed25519 ...".
set -euo pipefail
[ "$(id -u)" = 0 ] || { echo "run as root" >&2; exit 1; }
export DEBIAN_FRONTEND=noninteractive
U=glot

timedatectl set-timezone UTC
apt-get update -q
apt-get -yq upgrade
apt-get -yq install ca-certificates curl rsync ufw fail2ban unattended-upgrades

# --- Docker Engine + compose plugin (official repo) ---
if ! command -v docker >/dev/null; then
  install -m 0755 -d /etc/apt/keyrings
  curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o /etc/apt/keyrings/docker.asc
  chmod a+r /etc/apt/keyrings/docker.asc
  echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu $(. /etc/os-release && echo "$VERSION_CODENAME") stable" \
    > /etc/apt/sources.list.d/docker.list
  apt-get update -q
  apt-get -yq install docker-ce docker-ce-cli containerd.io docker-buildx-plugin docker-compose-plugin
fi
systemctl enable --now docker

# --- non-root user with the SSH key ---
id "$U" >/dev/null 2>&1 || adduser --disabled-password --gecos "" "$U"
usermod -aG docker,sudo "$U"
# docker group is root-equivalent anyway; passwordless sudo keeps admin possible once root login is off.
echo "$U ALL=(ALL) NOPASSWD:ALL" > /etc/sudoers.d/90-$U && chmod 440 /etc/sudoers.d/90-$U
install -d -m 700 -o "$U" -g "$U" /home/$U/.ssh
keys=/home/$U/.ssh/authorized_keys
touch "$keys"
if [ -n "${GLOT_SSH_PUBKEY:-}" ]; then grep -qxF "$GLOT_SSH_PUBKEY" "$keys" || echo "$GLOT_SSH_PUBKEY" >> "$keys"; fi
if [ ! -s "$keys" ] && [ -s /root/.ssh/authorized_keys ]; then cp /root/.ssh/authorized_keys "$keys"; fi
chown "$U:$U" "$keys" && chmod 600 "$keys"

# --- SSH: keys only, no root. Lockout guard: only once glot has a key. ---
if [ -s "$keys" ]; then
  cat > /etc/ssh/sshd_config.d/99-glot.conf <<CONF
PermitRootLogin no
PasswordAuthentication no
KbdInteractiveAuthentication no
PubkeyAuthentication yes
CONF
  sshd -t && systemctl reload ssh
else
  echo "WARNING: no SSH key for $U (set GLOT_SSH_PUBKEY): root/password login left untouched" >&2
fi

# --- firewall: 22, 80, 443 (+443/udp for HTTP/3). Docker only publishes 80/443 (Caddy). ---
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp
ufw allow 80/tcp
ufw allow 443/tcp
ufw allow 443/udp
ufw --force enable

# --- fail2ban (Ubuntu ships the sshd jail enabled) + automatic security updates ---
systemctl enable --now fail2ban
cat > /etc/apt/apt.conf.d/20auto-upgrades <<CONF
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
CONF
systemctl enable --now unattended-upgrades

# --- 2 GB swap (helps the Rust build on a 4 GB box) ---
if ! swapon --show=NAME --noheadings | grep -qx /swapfile; then
  [ -f /swapfile ] || { fallocate -l 2G /swapfile; chmod 600 /swapfile; mkswap /swapfile; }
  swapon /swapfile
fi
grep -q '^/swapfile ' /etc/fstab || echo '/swapfile none swap sw 0 0' >> /etc/fstab
echo 'vm.swappiness=10' > /etc/sysctl.d/99-glot.conf && sysctl -q --system

echo "done. Next: ssh $U@$(hostname -I | awk '{print $1}')  (root login is now disabled)"
