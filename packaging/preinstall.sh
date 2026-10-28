#!/bin/bash
set -e

case "${1:-}" in
  upgrade|install)
    if systemctl is-active --quiet feathertail.service 2>/dev/null; then
      systemctl stop feathertail.service || true
    fi
    ;;
esac

exit 0
