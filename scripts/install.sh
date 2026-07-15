#!/usr/bin/env bash
# Install or upgrade rusticlens (GUI + TUI) from the latest GitHub Release.
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/mfahmirukman/rusticlens/develop/scripts/install.sh | bash
#   # or from a clone:
#   ./scripts/install.sh
#   PREFIX=/usr/local/bin ./scripts/install.sh
#   FORCE=1 ./scripts/install.sh          # reinstall even if up to date
set -euo pipefail

REPO="${RUSTICLENS_REPO:-mfahmirukman/rusticlens}"
PREFIX="${PREFIX:-${HOME}/.local/bin}"
STATE_DIR="${XDG_STATE_HOME:-${HOME}/.local/state}/rusticlens"
VERSION_FILE="${STATE_DIR}/installed-version"
TMPDIR="${TMPDIR:-/tmp}"
WORK="$(mktemp -d "${TMPDIR%/}/rusticlens-install.XXXXXX")"
cleanup() { rm -rf "${WORK}"; }
trap cleanup EXIT

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "error: required command not found: $1" >&2
    exit 1
  }
}

need curl
need tar
need uname
need mkdir
need mktemp
need install
need rm
need mv
need chmod

os="$(uname -s)"
arch="$(uname -m)"
case "${os}" in
  Linux)
    case "${arch}" in
      x86_64|amd64) artifact="linux-x86_64"; archive_ext="tar.gz" ;;
      *)
        echo "error: unsupported Linux arch: ${arch} (releases ship linux-x86_64)" >&2
        exit 1
        ;;
    esac
    ;;
  Darwin)
    artifact="macos-universal"
    archive_ext="tar.gz"
    ;;
  *)
    echo "error: unsupported OS: ${os} (use GitHub Releases zip on Windows)" >&2
    exit 1
    ;;
esac

json_get_tag() {
  # Prefer python (common), else jq, else a minimal sed parse of tag_name.
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"])'
  elif command -v jq >/dev/null 2>&1; then
    jq -r .tag_name
  else
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1
  fi
}

echo "Fetching latest release from ${REPO}..."
latest_json="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest")"
latest_tag="$(printf '%s' "${latest_json}" | json_get_tag)"
if [[ -z "${latest_tag}" || "${latest_tag}" == "null" ]]; then
  echo "error: could not resolve latest release tag" >&2
  exit 1
fi
latest_ver="${latest_tag#v}"
echo "Latest release: ${latest_tag}"

existing_ver=""
if [[ -f "${VERSION_FILE}" ]]; then
  existing_ver="$(tr -d '[:space:]' <"${VERSION_FILE}")"
fi
# Fall back to probing binaries already on PATH / PREFIX.
if [[ -z "${existing_ver}" ]]; then
  for candidate in "${PREFIX}/rusticlens-tui" "${PREFIX}/rusticlens" \
    "$(command -v rusticlens-tui 2>/dev/null || true)" \
    "$(command -v rusticlens 2>/dev/null || true)"; do
    if [[ -n "${candidate}" && -x "${candidate}" ]]; then
      if out="$("${candidate}" --version 2>/dev/null)"; then
        existing_ver="$(printf '%s\n' "${out}" | head -1 | awk '{print $NF}' | tr -d 'v')"
        break
      fi
    fi
  done
fi

if [[ -n "${existing_ver}" ]]; then
  echo "Installed version: ${existing_ver}"
else
  echo "Installed version: (none)"
fi

if [[ -z "${FORCE:-}" && -n "${existing_ver}" && "${existing_ver}" == "${latest_ver}" ]]; then
  echo "Already up to date (${latest_ver}). Set FORCE=1 to reinstall."
  exit 0
fi

asset="rusticlens-${latest_ver}-${artifact}.${archive_ext}"
url="https://github.com/${REPO}/releases/download/${latest_tag}/${asset}"
echo "Downloading ${url}"
curl -fL --progress-bar -o "${WORK}/${asset}" "${url}"

# Optional checksum verification when SHA256SUMS.txt is present.
if sums="$(curl -fsSL "https://github.com/${REPO}/releases/download/${latest_tag}/SHA256SUMS.txt" 2>/dev/null || true)" \
  && [[ -n "${sums}" ]]; then
  printf '%s\n' "${sums}" >"${WORK}/SHA256SUMS.txt"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "${WORK}" && sha256sum -c --ignore-missing SHA256SUMS.txt)
  elif command -v shasum >/dev/null 2>&1; then
    expected="$(awk -v f="${asset}" '$2==f {print $1; exit}' "${WORK}/SHA256SUMS.txt")"
    actual="$(shasum -a 256 "${WORK}/${asset}" | awk '{print $1}')"
    [[ "${expected}" == "${actual}" ]] || {
      echo "error: checksum mismatch for ${asset}" >&2
      exit 1
    }
  fi
fi

tar -xzf "${WORK}/${asset}" -C "${WORK}"
[[ -f "${WORK}/rusticlens" && -f "${WORK}/rusticlens-tui" ]] || {
  echo "error: archive did not contain rusticlens and rusticlens-tui" >&2
  exit 1
}

if [[ "${os}" == "Darwin" ]]; then
  xattr -cr "${WORK}/rusticlens" "${WORK}/rusticlens-tui" 2>/dev/null || true
fi

mkdir -p "${PREFIX}" "${STATE_DIR}"

# Replace atomically: install to .new, then swap; remove previous binary after success.
for bin in rusticlens rusticlens-tui; do
  dest="${PREFIX}/${bin}"
  tmp="${dest}.new"
  old="${dest}.old"
  install -m 755 "${WORK}/${bin}" "${tmp}"
  if [[ -e "${dest}" ]]; then
    mv -f "${dest}" "${old}"
  fi
  mv -f "${tmp}" "${dest}"
  rm -f "${old}"
  echo "Installed ${dest}"
done

printf '%s\n' "${latest_ver}" >"${VERSION_FILE}"

case ":${PATH}:" in
  *":${PREFIX}:"*) ;;
  *)
    echo
    echo "Note: ${PREFIX} is not on PATH. Add this to your shell profile:"
    echo "  export PATH=\"${PREFIX}:\$PATH\""
    ;;
esac

echo
echo "rusticlens ${latest_ver} installed."
echo "  GUI: rusticlens"
echo "  TUI: rusticlens-tui"
echo
echo "Optional (Wayland clipboard for TUI yank/copy):"
echo "  Fedora/Nobara: sudo dnf install wl-clipboard"
echo "  Debian/Ubuntu: sudo apt install wl-clipboard"
echo "  Arch:          sudo pacman -S wl-clipboard"
echo "  X11 fallback:  xclip or xsel"
