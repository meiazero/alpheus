#!/bin/sh
# Installs the alpheus release binary for the current user:
#   curl -fsSL https://raw.githubusercontent.com/meiazero/alpheus/main/install.sh | sh
# ALPHEUS_VERSION pins a release (default: latest); ALPHEUS_INSTALL_DIR overrides ~/.local/bin.
# Everything runs inside main, so a download cut short never executes half a script.
set -eu

fail() {
    printf 'alpheus install: %s\n' "$1" >&2
    exit 1
}

main() {
    platform="$(uname -s) $(uname -m)"
    # ponytail: releases ship Linux x86_64 only; add a case per target once dist-workspace.toml builds it.
    case "$platform" in
        "Linux x86_64") target=x86_64-unknown-linux-musl ;;
        *) fail "no release binary for $platform; build from source: cargo install --git https://github.com/meiazero/alpheus" ;;
    esac
    for tool in curl tar xz sha256sum; do
        command -v "$tool" >/dev/null 2>&1 || fail "$tool is required"
    done

    base="${ALPHEUS_RELEASE_BASE:-https://github.com/meiazero/alpheus/releases}"
    protocol='=https'
    # Tests serve a local release over plain HTTP; nothing else may.
    case "$base" in
        http://127.0.0.1:|http://127.0.0.1:*[!0-9]*) ;;
        http://127.0.0.1:*) protocol='=http' ;;
    esac
    version="${ALPHEUS_VERSION:-}"
    if [ -n "$version" ]; then
        version="v${version#v}"
        url="$base/download/$version"
    else
        version=latest
        url="$base/latest/download"
    fi

    asset="alpheus-$target.tar.xz"
    tmp="$(mktemp -d)"
    trap 'rm -rf "$tmp"' EXIT
    for file in "$asset" sha256.sum; do
        curl --proto "$protocol" --tlsv1.2 -fsSL --retry 3 -o "$tmp/$file" "$url/$file" \
            || fail "cannot download $file for alpheus $version"
    done
    # A missing line leaves sha256sum with no input, which fails like a mismatch.
    (cd "$tmp" && awk -v asset="$asset" '$2 == asset' sha256.sum \
        | sha256sum --check --status -) \
        || fail "checksum mismatch for $asset; refusing to install it"
    tar -xJf "$tmp/$asset" -C "$tmp"

    dir="${ALPHEUS_INSTALL_DIR:-$HOME/.local/bin}"
    mkdir -p "$dir"
    # Copy next to the target, then rename, so a running alpheus is never overwritten mid-write.
    # The binary is never run here: it is static, so the platform check above is enough, and the
    # installer stays reviewable as a single stage.
    cp "$tmp/alpheus-$target/alpheus" "$dir/.alpheus.new"
    chmod 755 "$dir/.alpheus.new"
    mv -f "$dir/.alpheus.new" "$dir/alpheus"
    printf 'Installed alpheus %s to %s/alpheus\n' "$version" "$dir"

    case ":$PATH:" in
        *":$dir:"*) ;;
        *) printf 'Add %s to your PATH, e.g. in ~/.profile:\n  export PATH="%s:$PATH"\n' "$dir" "$dir" ;;
    esac
}

main "$@"
