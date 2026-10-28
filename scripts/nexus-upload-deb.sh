#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 ]]; then
  echo "Usage: $0 <path-to.deb>" >&2
  exit 1
fi

deb_file="$1"

if [[ ! -f "${deb_file}" ]]; then
  echo "File not found: ${deb_file}" >&2
  exit 1
fi

if [[ -z "${NEXUS_USERNAME:-}" || -z "${NEXUS_PASSWORD:-}" ]]; then
  echo "NEXUS_USERNAME and NEXUS_PASSWORD must be set." >&2
  exit 1
fi

NEXUS_HOST="${NEXUS_HOST:-https://apt.mythicalsystems.org}"
NEXUS_REPOSITORY="${NEXUS_REPOSITORY:-MythicalSystems}"
NEXUS_BASE="${NEXUS_BASE:-${NEXUS_HOST}/repository/${NEXUS_REPOSITORY}}"
NEXUS_MAX_RETRIES="${NEXUS_MAX_RETRIES:-5}"
NEXUS_RETRY_DELAY="${NEXUS_RETRY_DELAY:-15}"

filename="$(basename "${deb_file}")"
public_url="${NEXUS_BASE}/${filename}"
api_url="${NEXUS_HOST}/service/rest/v1/components?repository=${NEXUS_REPOSITORY}"

# Prod package must not use git-describe dirty versions (e.g. 1.0.4-5-gabcdef).
# Dev channel (~dev+) is allowed for feathertail-dev only.
if [[ "${filename}" == feathertail_* && "${filename}" != feathertail-dev_* ]]; then
  if [[ "${NEXUS_BLOCK_DEV_VERSIONS:-1}" == "1" && "${filename}" =~ feathertail_.*-g[0-9a-f]+_ ]]; then
    echo "Refusing to upload dirty git-describe version for prod package: ${filename}" >&2
    echo "Use a clean release tag version instead (e.g. 1.0.4)." >&2
    exit 3
  fi
fi

curl_auth=(-u "${NEXUS_USERNAME}:${NEXUS_PASSWORD}")

if [[ "${NEXUS_CHECK_EXISTS:-0}" == "1" ]]; then
  if curl -sf "${curl_auth[@]}" -I "${public_url}" >/dev/null 2>&1; then
    echo "Skipping ${filename} (already present in Nexus)"
    exit 2
  fi
fi

echo "Uploading ${filename} to ${NEXUS_REPOSITORY} (POST ${api_url})"

attempt=1
delay="${NEXUS_RETRY_DELAY}"
while [[ "${attempt}" -le "${NEXUS_MAX_RETRIES}" ]]; do
  http_code="$(curl -sS -o /tmp/nexus-upload-response.txt -w '%{http_code}' \
    "${curl_auth[@]}" \
    -X POST "${api_url}" \
    -H 'accept: application/json' \
    -F "apt.asset=@${deb_file}")"

  if [[ "${http_code}" -ge 200 && "${http_code}" -lt 300 ]]; then
    echo "Successfully uploaded ${filename} (HTTP ${http_code})"
    exit 0
  fi

  # Idempotent re-runs / race between tag-push and release-published jobs.
  if [[ "${http_code}" =~ ^(400|409)$ ]]; then
    response="$(cat /tmp/nexus-upload-response.txt 2>/dev/null || true)"
    if echo "${response}" | grep -Eiq 'already exists|repository does not allow updating assets|cannot be updated'; then
      echo "Skipping ${filename} (already present in Nexus; HTTP ${http_code})"
      exit 2
    fi
  fi

  if [[ "${http_code}" =~ ^(429|502|503|504)$ && "${attempt}" -lt "${NEXUS_MAX_RETRIES}" ]]; then
    echo "Transient error (HTTP ${http_code}). Waiting ${delay}s before retry ${attempt}/${NEXUS_MAX_RETRIES}..." >&2
    sleep "${delay}"
    delay=$((delay * 2))
    attempt=$((attempt + 1))
    continue
  fi

  echo "Failed to upload ${filename} (HTTP ${http_code})" >&2
  cat /tmp/nexus-upload-response.txt >&2
  exit 1
done
