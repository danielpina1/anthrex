# Installing anthrex

anthrex ships as a single `anthrex` binary for Linux and macOS. Each
[GitHub release](https://github.com/danielpina1/anthrex/releases) has one archive per
platform, plus a `.sha256` checksum file for each archive.

anthrex runs the `claude` and `codex` apps you already have installed; it does not
install them for you.

| Platform | Target |
|----------|--------|
| Linux, x86_64 | `x86_64-unknown-linux-gnu` |
| Linux, ARM64 | `aarch64-unknown-linux-gnu` |
| macOS, Apple silicon | `aarch64-apple-darwin` |
| macOS, Intel | `x86_64-apple-darwin` |

## 1. Download

Pick your target from the table and download its archive and checksum. For 0.1.0:

```bash
VERSION=v0.1.0
TARGET=aarch64-apple-darwin   # or x86_64-apple-darwin, x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu
BASE=https://github.com/danielpina1/anthrex/releases/download/$VERSION

curl -fLO "$BASE/anthrex-$VERSION-$TARGET.tar.gz"
curl -fLO "$BASE/anthrex-$VERSION-$TARGET.tar.gz.sha256"
```

Release pages list every archive; the newest one is always at
<https://github.com/danielpina1/anthrex/releases/latest>. While anthrex is in alpha,
releases are marked as pre-releases.

## 2. Verify the checksum

```bash
# macOS
shasum -a 256 -c "anthrex-$VERSION-$TARGET.tar.gz.sha256"
# Linux
sha256sum -c "anthrex-$VERSION-$TARGET.tar.gz.sha256"
```

Both print `anthrex-<version>-<target>.tar.gz: OK`. Do not use the binary if they
report a mismatch.

## 3. Unpack and put `anthrex` on your PATH

The archive holds `anthrex`, `README.md` and, when present, `LICENSE`.

```bash
mkdir anthrex-unpacked
tar -xzf "anthrex-$VERSION-$TARGET.tar.gz" -C anthrex-unpacked

mkdir -p ~/.local/bin
install -m 755 anthrex-unpacked/anthrex ~/.local/bin/anthrex
```

If `~/.local/bin` is not on your PATH yet, add it to your shell's startup file
(`~/.zshrc` or `~/.bashrc`):

```bash
export PATH="$HOME/.local/bin:$PATH"
```

To install system-wide instead, use `sudo install -m 755 anthrex-unpacked/anthrex /usr/local/bin/anthrex`.

Check the install:

```bash
anthrex --version   # anthrex 0.1.0
```

## macOS: Gatekeeper

The binaries are not signed or notarized, so macOS quarantines a downloaded copy
and refuses to run it ("cannot be opened because the developer cannot be verified").
Clear the quarantine flag once after installing:

```bash
xattr -d com.apple.quarantine ~/.local/bin/anthrex
```

Or run `anthrex` once, then open **System Settings → Privacy & Security** and click
**Allow Anyway** next to the message about `anthrex`.

Downloading with `curl` usually does not set the quarantine flag; a browser download
does. If `xattr` says there is no such attribute, there is nothing to do.

## Building from source

You need Rust 1.92 or newer ([rustup](https://rustup.rs)) and git.

```bash
git clone https://github.com/danielpina1/anthrex.git
cd anthrex
git checkout v0.1.0          # or stay on main for the latest code
cargo build --release --locked -p anthrex
install -m 755 target/release/anthrex ~/.local/bin/anthrex
```

Or let cargo install it into `~/.cargo/bin`:

```bash
cargo install --locked --git https://github.com/danielpina1/anthrex --tag v0.1.0 anthrex
```

## Upgrading and uninstalling

To upgrade, repeat the steps above with the new version and stop the old daemon so
the new binary starts a fresh one: `anthrex daemon stop`. Your agents' sessions are
kept and resume on the next start.

To uninstall, run `anthrex daemon stop` and delete the binary.
