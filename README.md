# CanYouRunLLM Detector

A tiny, open-source hardware detector for [CanYouRunLLM.com](https://canyourunllm.vercel.app). Double-click it and it opens your browser to a page showing which local LLMs your PC can run, how fast, and what's holding it back.

## What it reads

Only hardware and OS information:

- **GPU:** name, VRAM, memory bandwidth (via DXGI on Windows, `nvidia-smi`, `lspci`, or `sysctl` on macOS)
- **CPU:** model name, core/thread count, AVX2/AVX-512 support
- **RAM:** total system memory (rounded to GB)
- **Disk:** total and free space, drive type
- **OS:** operating system name/version

It does **not** read your username, hostname, files, serial numbers, network info, or anything personal.

## What it does with it

The specs are serialized to JSON, base64url-encoded, and the detector opens your default browser at:

```
https://canyourunllm.vercel.app/my-pc?spec=<base64>
```

That's it. The detector makes **no network calls of its own**; no telemetry, no uploads, no auto-updates. The only thing that leaves your machine is that URL, opened by your browser. You can decode the `spec` parameter yourself to see exactly what was sent.

## Build

Requires [Rust](https://rustup.rs) (stable).

```sh
cargo build --release
# binary: target/release/canyourunllm-detector(.exe)
```

To point it at a different site at build time:

```sh
CANYOURUNLLM_URL=https://example.com cargo build --release
```

Windows release builds run without a console window.

## CI

`.github/workflows/build.yml` builds the Windows x86_64 `.exe` on `windows-latest` and uploads it as a build artifact. Release binaries are built in CI so they can be code-signed.

## License

MIT © 2026 Darren Liu
