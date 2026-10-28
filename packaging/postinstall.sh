#!/bin/bash
set -e

CONFIG="/etc/feathertail/feathertail.toml"
BINARY="/usr/local/bin/feathertail"
SERVICE_UNIT="/lib/systemd/system/feathertail.service"
LEGACY_SERVICE="/etc/systemd/system/feathertail.service"

case "${1:-}" in
  configure)
    ;;
  *)
    exit 0
    ;;
esac

mkdir -p /etc/feathertail /var/lib/feathertail
chmod 755 /etc/feathertail /var/lib/feathertail

# Prefer the packaged unit over a previous binary service-install unit.
if [ -f "${SERVICE_UNIT}" ] && [ -f "${LEGACY_SERVICE}" ]; then
  rm -f "${LEGACY_SERVICE}"
fi

systemctl daemon-reload || true

if [ ! -f "${CONFIG}" ]; then
  echo ""
  echo "FeatherTail installed, but no config was found at ${CONFIG}."
  echo "Create one, then:"
  echo "  sudo systemctl enable --now feathertail"
  echo ""
  exit 0
fi

if [ ! -d /etc/pve ]; then
  echo ""
  echo "FeatherTail installed. This host does not look like Proxmox VE (/etc/pve missing)."
  echo "The service will stay inactive until installed on a Proxmox host."
  echo "Edit ${CONFIG}, then: sudo systemctl enable --now feathertail"
  echo ""
  exit 0
fi

systemctl enable feathertail.service >/dev/null 2>&1 || true
if systemctl is-enabled --quiet feathertail.service 2>/dev/null; then
  systemctl restart feathertail.service >/dev/null 2>&1 || systemctl start feathertail.service >/dev/null 2>&1 || true
else
  systemctl start feathertail.service >/dev/null 2>&1 || true
fi

echo "FeatherTail installed. Configuration: ${CONFIG}"
echo "Binary: ${BINARY}"
echo "Service: systemctl status feathertail"

exit 0
