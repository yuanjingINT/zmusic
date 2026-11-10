#!/bin/sh
# zmuse installer — one line, no root:
#
#   curl -fsSL https://repo.dito.asia/zmuse/install.sh | sh
#
# Installs a prebuilt static binary into ~/.local/bin (override with --prefix
# or ZMUSE_PREFIX). Nothing outside your home directory is touched, and the
# script never runs sudo. Re-run it to upgrade; --uninstall removes the binary.
#
# Options:
#   --prefix DIR   install into DIR (default ~/.local/bin or $ZMUSE_PREFIX)
#   --version X    install a specific version instead of "latest"
#   --uninstall    remove the installed binary
#   --insecure-sha-only  install without gpg (sha256 from SHA256SUMS still checked)
#   --no-sidecar   do not install the Meting sidecar / systemd user unit
#   --help
#
# Download layout (published by the release job):
#   <base>/latest                      plain-text version, atomically replaced
#   <base>/v<ver>/SHA256SUMS           one line per artifact
#   <base>/v<ver>/SHA256SUMS.sig       detached signature of SHA256SUMS
#   <base>/v<ver>/zmuse-<ver>-linux-<arch>.tar.gz
#   <base>/v<ver>/zmuse-<ver>-linux-<arch>.tar.gz (unpacked: zmuse-<ver>-linux-<arch>/zmuse)
#
# Environment (mirrors / CI):
#   ZMUSE_INSTALL_BASE  base URL of the download directory
#   ZMUSE_KEY_URL       the repository public key
#   ZMUSE_PREFIX        same as --prefix
set -eu

BASE="${ZMUSE_INSTALL_BASE:-https://repo.dito.asia/zmuse}"
KEY_URL="${ZMUSE_KEY_URL:-https://repo.dito.asia/x86_64/dito-repo.pub}"
# Pinned repository key. Do not make this configurable: it is the whole point.
EXPECTED_FP="6469F0BEB079C3EDCC47947627FEDD0D74F7D714"

PREFIX="${ZMUSE_PREFIX:-$HOME/.local/bin}"
VERSION=""
DO_UNINSTALL=0
DO_GPG=1
DO_SIDECAR=1

die() { printf 'zmuse: %s\n' "$*" >&2; exit 1; }
say() { printf '%s\n' "$*"; }

usage() {
    sed -n '2,26p' "$0" 2>/dev/null | sed 's/^# \{0,1\}//' || true
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)    [ $# -ge 2 ] || die "--prefix needs a directory"; PREFIX="$2"; shift 2 ;;
        --version)   [ $# -ge 2 ] || die "--version needs a value"; VERSION="$2"; shift 2 ;;
        --uninstall) DO_UNINSTALL=1; shift ;;
        --no-gpg|--insecure-sha-only) DO_GPG=0; shift ;;
        --no-sidecar) DO_SIDECAR=0; shift ;;
        --help|-h)   usage ;;
        *)           die "unknown option: $1 (try --help)" ;;
    esac
done

PREFIX="${PREFIX/#\~/$HOME}"
BIN="$PREFIX/zmuse"

# ---------------------------------------------------------------- uninstall
if [ "$DO_UNINSTALL" -eq 1 ]; then
    if [ -e "$BIN" ]; then
        rm -f "$BIN"
        say "removed $BIN"
    else
        say "nothing to remove at $BIN"
    fi
    if [ -f "$HOME/.config/systemd/user/zmuse-meting.service" ]; then
        if command -v systemctl >/dev/null 2>&1; then
            systemctl --user disable --now zmuse-meting.service >/dev/null 2>&1 || true
        fi
        rm -f "$HOME/.config/systemd/user/zmuse-meting.service"
        command -v systemctl >/dev/null 2>&1 && systemctl --user daemon-reload >/dev/null 2>&1 || true
        say "removed the Meting sidecar unit"
    fi
    rm -rf "$HOME/.local/share/zmuse"
    say "your settings were left alone: ~/.config/zmuse and ~/.cache/zmuse"
    exit 0
fi

# ---------------------------------------------------------------- platform
[ "$(uname -s)" = "Linux" ] || die "this installer handles Linux only right now (found $(uname -s))"
case "$(uname -m)" in
    x86_64|amd64)   ARCH=x86_64 ;;
    aarch64|arm64)  ARCH=aarch64 ;;
    *)              die "unsupported architecture: $(uname -m)" ;;
esac

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar  >/dev/null 2>&1 || die "tar is required"

# ---------------------------------------------------------------- version
if [ -z "$VERSION" ]; then
    VERSION="$(curl -fsSL "$BASE/latest" 2>/dev/null | tr -d ' \t\r\n')" ||
        die "could not read $BASE/latest"
    [ -n "$VERSION" ] || die "could not read $BASE/latest (empty)"
fi
case "$VERSION" in
    *[!0-9A-Za-z._-]*) die "odd version string: $VERSION" ;;
esac

NAME="zmuse-$VERSION-linux-$ARCH"
TARBALL="$NAME.tar.gz"
VDIR="$BASE/v$VERSION"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/zmuse-install.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT INT TERM

say "zmuse $VERSION ($ARCH) -> $PREFIX"

# ---------------------------------------------------------------- checksum list
# 先拿 SHA256SUMS 和它的签名：清单是被签名的那一份，tarball 的哈希只从清单里取。
curl -fsSL -o "$TMP/SHA256SUMS" "$VDIR/SHA256SUMS" ||
    die "could not fetch $VDIR/SHA256SUMS"
WANT="$(awk -v f="$TARBALL" '$2 == f || $2 == "*"f {print $1; exit}' "$TMP/SHA256SUMS")"
[ -n "$WANT" ] || die "$TARBALL is not listed in SHA256SUMS (unknown version $VERSION for $ARCH?)"

# ---------------------------------------------------------------- signature
if [ "$DO_GPG" -eq 1 ]; then
    if ! command -v gpg >/dev/null 2>&1; then
        die "gpg is required to verify the signature; install gnupg, or pass --insecure-sha-only"
    fi
    curl -fsSL -o "$TMP/SHA256SUMS.sig" "$VDIR/SHA256SUMS.sig" ||
        die "signature SHA256SUMS.sig is missing; refusing to install unverified (use --insecure-sha-only)"
    curl -fsSL -o "$TMP/repo.pub" "$KEY_URL" ||
        die "could not fetch the repository key from $KEY_URL"
    # Throwaway keyring: the user's ~/.gnupg is never modified.
    export GNUPGHOME="$TMP/gnupg"
    mkdir -p "$GNUPGHOME"
    chmod 700 "$GNUPGHOME"
    gpg --batch --quiet --import "$TMP/repo.pub" >/dev/null 2>&1 ||
        die "could not import the repository key"
    FPS="$(gpg --batch --with-colons --fingerprint 2>/dev/null | awk -F: '/^fpr:/{print $10; exit}')"
    [ "$FPS" = "$EXPECTED_FP" ] ||
        die "repository key fingerprint mismatch: got $FPS, expected $EXPECTED_FP"
    gpg --batch --quiet --verify "$TMP/SHA256SUMS.sig" "$TMP/SHA256SUMS" >/dev/null 2>&1 ||
        die "signature verification failed for SHA256SUMS"
    say "  signature ok ($EXPECTED_FP)"
else
    say "  WARNING: signature check skipped (--insecure-sha-only)"
fi

# ---------------------------------------------------------------- download
say "  downloading $TARBALL"
curl -fsSL -o "$TMP/$TARBALL" "$VDIR/$TARBALL" || die "download failed: $VDIR/$TARBALL"

if command -v sha256sum >/dev/null 2>&1; then
    GOT="$(sha256sum "$TMP/$TARBALL" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
    GOT="$(shasum -a 256 "$TMP/$TARBALL" | awk '{print $1}')"
else
    die "no sha256 tool (need sha256sum or shasum)"
fi
[ "$GOT" = "$WANT" ] || die "sha256 mismatch: expected $WANT, got $GOT"
say "  sha256 ok"

# ---------------------------------------------------------------- install
tar -xzf "$TMP/$TARBALL" -C "$TMP" || die "could not unpack $TARBALL"
SRC="$TMP/$NAME/zmuse"
[ -f "$SRC" ] || SRC="$(find "$TMP/$NAME" -maxdepth 2 -name zmuse -type f 2>/dev/null | head -1)"
[ -n "${SRC:-}" ] && [ -f "$SRC" ] || die "no zmuse binary inside $TARBALL"

mkdir -p "$PREFIX"
if [ -e "$BIN" ]; then
    OLD="$("$BIN" --version 2>/dev/null || echo unknown)"
    say "  replacing $OLD"
fi
cp -f "$SRC" "$BIN.new"
chmod 755 "$BIN.new"
mv -f "$BIN.new" "$BIN"
say "  installed $BIN"

# ---------------------------------------------------------------- check
GOT_VER="$("$BIN" --version 2>/dev/null || true)"
case "$GOT_VER" in
    *"$VERSION"*) say "  $GOT_VER" ;;
    "")           die "the binary does not run on this machine (glibc/musl mismatch?)" ;;
    *)            die "unexpected version: $GOT_VER" ;;
esac

# ---------------------------------------------------------------- sidecar + docs
SIDE_DST="$HOME/.local/share/zmuse/sidecar"
UNIT_DST="$HOME/.config/systemd/user/zmuse-meting.service"
if [ "$DO_SIDECAR" -eq 1 ] && [ -f "$TMP/$NAME/sidecar/meting.js" ]; then
    mkdir -p "$SIDE_DST"
    cp -a "$TMP/$NAME/sidecar/." "$SIDE_DST/"
    say "  sidecar -> $SIDE_DST"
    if [ -f "$TMP/$NAME/zmuse-meting.service" ]; then
        mkdir -p "$(dirname "$UNIT_DST")"
        install -m 644 "$TMP/$NAME/zmuse-meting.service" "$UNIT_DST"
        say "  unit    -> $UNIT_DST"
    fi
    if command -v systemctl >/dev/null 2>&1; then
        systemctl --user daemon-reload >/dev/null 2>&1 || true
    fi
elif [ "$DO_SIDECAR" -eq 1 ]; then
    say ""
    say "note: this tarball has no Meting sidecar, so QQ / Kugou / Baidu / Kuwo will"
    say "      report \"sidecar not running\". Everything else works."
fi

# 文档和许可证
DOC_DST="$HOME/.local/share/doc/zmuse"
for f in README.md CONFIG.md LICENSE; do
    if [ -f "$TMP/$NAME/$f" ]; then
        mkdir -p "$DOC_DST"
        install -m 644 "$TMP/$NAME/$f" "$DOC_DST/$f"
    fi
done

# ---------------------------------------------------------------- runtime deps
if ! command -v mpv >/dev/null 2>&1; then
    say ""
    say "mpv is required for playback. Install it with:"
    if [ -r /etc/os-release ]; then
        . /etc/os-release
        case "${ID:-}${ID_LIKE:-}" in
            *arch*)                 say "  sudo pacman -S mpv" ;;
            *debian*|*ubuntu*|*linuxmint*) say "  sudo apt install mpv" ;;
            *fedora*)               say "  sudo dnf install mpv" ;;
            *opensuse*|*suse*)      say "  sudo zypper install mpv" ;;
            *alpine*)               say "  sudo apk add mpv" ;;
            *void*)                 say "  sudo xbps-install mpv" ;;
            *)                      say "  install mpv with your package manager" ;;
        esac
    else
        say "  install mpv with your package manager"
    fi
fi
if ! command -v node >/dev/null 2>&1; then
    say ""
    say "node >= 22 is optional: it powers the local NetEase API and the Meting"
    say "sidecar (QQ / Kugou / Baidu / Kuwo). zmuse starts without it, with only"
    say "the sources that need no sidecar."
fi

case ":$PATH:" in
    *":$PREFIX:"*) ;;
    *)
        say ""
        say "$PREFIX is not in your PATH. Add it:"
        say "  echo 'export PATH=\"$PREFIX:\$PATH\"' >> ~/.profile"
        say "and open a new shell (or export it for this one)."
        ;;
esac

if command -v zmuse >/dev/null 2>&1; then
    SYS="$(command -v zmuse)"
    if [ "$SYS" != "$BIN" ]; then
        say ""
        say "note: your PATH resolves zmuse to $SYS first, so that one runs."
    fi
fi

if [ "$DO_SIDECAR" -eq 1 ] && [ -f "$UNIT_DST" ] && command -v node >/dev/null 2>&1; then
    if ! systemctl --user is-enabled zmuse-meting.service >/dev/null 2>&1; then
        say ""
        say "Enable the Meting sidecar (QQ / Kugou / Baidu / Kuwo sources):"
        say "  systemctl --user enable --now zmuse-meting.service"
    fi
fi

say ""
say "done. run: zmuse"
