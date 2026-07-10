# @ferrimock/cli

Native `ferrimock` CLI distributed through npm. Installing this package pulls
the prebuilt binary for your platform as an optionalDependency and exposes
it as the `ferrimock` command - no postinstall step, no download at install
time.

```sh
npm install -g @ferrimock/cli
ferrimock --help
```

Supported platforms: macOS (arm64, x64), Linux (x64, arm64 - static musl
binaries that also run on glibc systems), Windows (x64). On any other
platform install from source:

```sh
cargo install @ferrimock/cli --locked
```

The full documentation lives in the
[ferrimock repository](https://github.com/salamaashoush/ferrimock).
