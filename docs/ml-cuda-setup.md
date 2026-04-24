# CUDA setup for Keepsake ML

ONNX Runtime's CUDA Execution Provider fails to register **silently**
when any of its dynamic-library dependencies can't be resolved. When
that happens, sessions fall back to CPU but `MlRuntime::load` returns
`Ok(_)` and `provider_label()` still reads `"Cuda"` — so the top-nav
badge lies to you.

This doc is the exact set of things that have to be true before you
get actual GPU inference.

## What you need

1. An NVIDIA GPU with a recent driver (RTX 4070 on 590.48.01 +
   CUDA 13.1 driver stack works — older Driver 525+ is fine for
   CUDA 12.x runtime).
2. **Seven** CUDA runtime libraries that `libonnxruntime_providers_cuda.so`
   dlopen's at session-build time. `ldd` on the provider lib enumerates
   them; verify they all resolve under your `LD_LIBRARY_PATH`:

   ```sh
   LD_LIBRARY_PATH=/path/to/cuda-libs \
     ldd vendor/onnxruntime-linux-x64-gpu-1.22.0/lib/libonnxruntime_providers_cuda.so \
     | grep 'not found'
   ```

   Zero lines = good. Any `not found` = CPU fallback incoming.

3. For ORT 1.22, the pins are:
   - `libcudart.so.12`
   - `libcublas.so.12` + `libcublasLt.so.12`
   - `libcufft.so.11`
   - `libcurand.so.10`
   - `libcudnn.so.9` (plus its split-graph friends on cuDNN 9)
   - **`libnvrtc.so.12`** — easy to miss; NVIDIA's redist ships
     it in the `cuda_nvrtc` sub-package, not the main `cuda-libs`
     tarball. The pip package `nvidia-cuda-nvrtc-cu12` bundles it
     at `~/.local/lib/python3.12/site-packages/nvidia/cuda_nvrtc/lib/`.

## Bootstrap (one-time)

The repo expects `vendor/cuda-libs/` to contain all seven libs. If
you downloaded a CUDA Toolkit tarball, `libnvrtc.so.12` may not be in
it — install the pip redist and symlink:

```sh
pip install --user nvidia-cuda-nvrtc-cu12
ln -s ~/.local/lib/python3.12/site-packages/nvidia/cuda_nvrtc/lib/libnvrtc.so.12 \
      vendor/cuda-libs/libnvrtc.so.12
```

(vendor/ is gitignored — this is a per-machine step.)

## Launching

```sh
ORT_DYLIB_PATH=$(pwd)/vendor/onnxruntime-linux-x64-gpu-1.22.0/lib/libonnxruntime.so \
LD_LIBRARY_PATH=$(pwd)/vendor/onnxruntime-linux-x64-gpu-1.22.0/lib:$(pwd)/vendor/cuda-libs \
MV_MODELS=~/.local/share/media-vault/models \
cargo tauri dev --features mv-app/ml-cuda
```

## Verifying GPU is actually in use

After unlocking the vault:

```sh
nvidia-smi --query-compute-apps=pid,process_name,used_memory --format=csv
```

A healthy GPU-backed session looks like:

```
pid, process_name, used_gpu_memory [MiB]
65434, /path/to/mv-app, 3556 MiB
```

~3.5 GiB is typical on 8 GiB cards: CLIP visual (~1.5 GiB CUDA
workspace) + textual (~0.5 GiB) + SCRFD (~0.3 GiB) + ArcFace
(~0.2 GiB) + cuDNN plan workspaces.

If `mv-app` doesn't appear in `nvidia-smi` but the badge says
`ML Cuda · idle`, CUDA EP registration silently failed. Re-run the
`ldd` check above.

## Expected inference speed (RTX 4070 vs 8-core CPU)

| Operation | CPU | RTX 4070 | Speedup |
|---|---|---|---|
| CLIP ViT-L/14 visual embed | ~800 ms | ~15 ms | ~50× |
| SCRFD 10g face detect | ~200 ms | ~8 ms | ~25× |
| ArcFace embed | ~50 ms | ~3 ms | ~17× |
| Reindex 2 000 assets end-to-end | 30–45 min | 1–2 min | ~20× |
