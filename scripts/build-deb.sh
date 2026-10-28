#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT}"

OUT_DIR="${OUT_DIR:-dist}"
# sqlite via sqlx needs a native toolchain; ship amd64 from CI for now.
ARCHES="${ARCHES:-amd64}"
PACKAGE_CHANNEL="${PACKAGE_CHANNEL:-prod}"
CARGO_PROFILE="${CARGO_PROFILE:-release}"
BASE_VERSION="${BASE_VERSION:-}"

if [[ -z "${BASE_VERSION}" ]]; then
  if grep -qE '^version = "' Cargo.toml; then
    BASE_VERSION="$(grep -m1 -E '^version = "' Cargo.toml | sed -E 's/^version = "([^"]+)".*/\1/')"
  elif git describe --tags --abbrev=0 >/dev/null 2>&1; then
    BASE_VERSION="$(git describe --tags --abbrev=0 | sed 's/^v//')"
  else
    BASE_VERSION="0.0.0"
  fi
fi

if [[ -z "${VERSION:-}" ]]; then
  if [[ "${PACKAGE_CHANNEL}" == "dev" ]]; then
    short_sha="$(git rev-parse --short HEAD 2>/dev/null || echo local)"
    stamp="$(date -u +%Y%m%d%H%M%S)"
    VERSION="${BASE_VERSION}~dev+${stamp}.${short_sha}"
  elif git describe --tags --exact-match >/dev/null 2>&1; then
    VERSION="$(git describe --tags --exact-match | sed 's/^v//')"
  else
    VERSION="${BASE_VERSION}"
  fi
fi

VERSION="${VERSION#v}"

case "${PACKAGE_CHANNEL}" in
  dev)
    NFPM_NAME="feathertail-dev"
    NFPM_CONFLICT="feathertail"
    ;;
  prod|*)
    NFPM_NAME="feathertail"
    NFPM_CONFLICT="feathertail-dev"
    ;;
esac

rust_target_for_arch() {
  case "$1" in
    amd64|x86_64) echo "x86_64-unknown-linux-gnu" ;;
    arm64|aarch64) echo "aarch64-unknown-linux-gnu" ;;
    *)
      echo "Unsupported arch: $1" >&2
      exit 1
      ;;
  esac
}

binary_arch_label() {
  case "$1" in
    amd64|x86_64) echo "amd64" ;;
    arm64|aarch64) echo "arm64" ;;
    *) echo "$1" ;;
  esac
}

ensure_nfpm() {
  if command -v nfpm >/dev/null 2>&1; then
    return 0
  fi

  echo "nfpm not found; installing from Goreleaser apt repo..."
  if ! command -v sudo >/dev/null 2>&1; then
    echo "nfpm is required. Install it from https://nfpm.goreleaser.com/install/" >&2
    exit 1
  fi

  echo 'deb [trusted=yes] https://repo.goreleaser.com/apt/ /' | sudo tee /etc/apt/sources.list.d/goreleaser.list >/dev/null
  sudo apt-get update
  sudo apt-get install -y nfpm
}

mkdir -p "${OUT_DIR}"
ensure_nfpm

VERSION="${VERSION}" ./scripts/generate-deb-metadata.sh

host_deb_arch="$(dpkg --print-architecture 2>/dev/null || uname -m)"
case "${host_deb_arch}" in
  x86_64) host_deb_arch=amd64 ;;
  aarch64) host_deb_arch=arm64 ;;
esac

echo "Building FeatherTail ${VERSION} (${NFPM_NAME}) for: ${ARCHES}"

: > "${OUT_DIR}/.deb-manifest"

cargo_profile_flag=()
if [[ "${CARGO_PROFILE}" == "release" ]]; then
  cargo_profile_flag=(--release)
elif [[ "${CARGO_PROFILE}" != "debug" ]]; then
  cargo_profile_flag=("--profile" "${CARGO_PROFILE}")
fi

for arch in ${ARCHES}; do
  target="$(rust_target_for_arch "${arch}")"
  label="$(binary_arch_label "${arch}")"

  echo ""
  echo "==> ${arch} (${target})"

  if [[ "${arch}" == "${host_deb_arch}" ]]; then
    cargo build "${cargo_profile_flag[@]}"
    built="target/${CARGO_PROFILE}/feathertail"
  else
    rustup target add "${target}" >/dev/null 2>&1 || true
    cargo build "${cargo_profile_flag[@]}" --target "${target}"
    built="target/${target}/${CARGO_PROFILE}/feathertail"
  fi

  if [[ ! -f "${built}" ]]; then
    echo "Built binary not found at ${built}" >&2
    exit 1
  fi

  cp -f "${built}" "${OUT_DIR}/feathertail"
  chmod 755 "${OUT_DIR}/feathertail"
  cp -f "${OUT_DIR}/feathertail" "${OUT_DIR}/feathertail-linux-${label}"
  chmod 755 "${OUT_DIR}/feathertail-linux-${label}"

  NFPM_NAME="${NFPM_NAME}" \
  NFPM_CONFLICT="${NFPM_CONFLICT}" \
  NFPM_ARCH="${arch}" \
  NFPM_VERSION="${VERSION}" \
  nfpm pkg \
    --packager deb \
    --config nfpm.yaml \
    --target "${OUT_DIR}/"

  echo "${OUT_DIR}/${NFPM_NAME}_${VERSION}_${arch}.deb" >> "${OUT_DIR}/.deb-manifest"
done

echo "${VERSION}" > "${OUT_DIR}/.deb-version"
echo "${NFPM_NAME}" > "${OUT_DIR}/.deb-package"

echo ""
echo "Built packages:"
ls -lh "${OUT_DIR}"/*.deb "${OUT_DIR}"/feathertail-linux-* 2>/dev/null || true

if command -v sha256sum >/dev/null 2>&1; then
  (
    cd "${OUT_DIR}"
    sha256sum feathertail-linux-* *.deb > checksums.txt
  )
  echo ""
  echo "Checksums written to ${OUT_DIR}/checksums.txt"
fi
