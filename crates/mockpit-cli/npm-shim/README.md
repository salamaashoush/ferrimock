# @mockpit/cli

Native `mockpit` CLI distributed through npm. Installing this package pulls
the prebuilt binary for your platform as an optionalDependency and exposes
it as the `mockpit` command - no postinstall step, no download at install
time.

```sh
npm install -g @mockpit/cli
mockpit --help
```

Supported platforms: macOS (arm64, x64), Linux (x64, arm64 - static musl
binaries that also run on glibc systems), Windows (x64). On any other
platform install from source:

```sh
cargo install mockpit-cli --locked
```

The full documentation lives in the
[mockpit repository](https://github.com/salamaashoush/mockpit).
