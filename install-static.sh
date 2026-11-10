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
#   --no-gpg       skip the signature check (sha256 is always checked)
#   --help
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
        --no-gpg)    DO_GPG=0; shift ;;
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
TMP="$(mktemp -d "${TMPDIR:-/tmp}/zmuse-install.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT INT TERM

say "zmuse $VERSION ($ARCH) -> $PREFIX"

# ---------------------------------------------------------------- download
say "  downloading $TARBALL"
curl -fsSL -o "$TMP/$TARBALL" "$BASE/$TARBALL" || die "download failed: $BASE/$TARBALL"

# ---------------------------------------------------------------- checksum
WANT="$(curl -fsSL "$BASE/$TARBALL.sha256" 2>/dev/null | awk '{print $1}' | head -1)" ||
    die "could not fetch $TARBALL.sha256"
[ -n "$WANT" ] || die "empty checksum file"
if command -v sha256sum >/dev/null 2>&1; then
    GOT="$(sha256sum "$TMP/$TARBALL" | awk '{print $1}')"
elif command -v shasum >/dev/null 2>&1; then
    GOT="$(shasum -a 256 "$TMP/$TARBALL" | awk '{print $1}')"
else
    die "no sha256 tool (need sha256sum or shasum)"
fi
[ "$GOT" = "$WANT" ] || die "sha256 mismatch: expected $WANT, got $GOT"
say "  sha256 ok"

# ---------------------------------------------------------------- signature
if [ "$DO_GPG" -eq 1 ]; then
    if ! command -v gpg >/dev/null 2>&1; then
        say "  gpg not installed, skipping the signature check (sha256 was verified)"
    else
        curl -fsSL -o "$TMP/tarball.sig" "$BASE/$TARBALL.sig" 2>/dev/null ||
            die "signature $TARBALL.sig is missing; refusing to install unverified (use --no-gpg to override)"
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
        gpg --batch --quiet --verify "$TMP/tarball.sig" "$TMP/$TARBALL" >/dev/null 2>&1 ||
            die "signature verification failed for $TARBALL"
        say "  signature ok ($EXPECTED_FP)"
    fi
fi

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

say ""
say "done. run: zmuse"
