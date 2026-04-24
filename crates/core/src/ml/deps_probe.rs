//! Runtime probe for ML execution-provider libraries.
//!
//! ONNX Runtime's execution providers fail to register **silently** when their
//! dynamic-library dependencies don't resolve. A session still builds, but
//! inference runs on CPU while `provider_label()` (in earlier versions)
//! happily reported `"Cuda"`. This module fixes that by performing a
//! pre-flight `dlopen` of every dylib each provider needs, so the loader can
//! report the provider that will *actually* be used — and warn the operator
//! when a GPU-capable build silently falls back.
//!
//! The canonical CUDA dep list is re-exported by `ort` as
//! [`ort::execution_providers::cuda::CUDA_DYLIBS`]. We forward it directly so
//! the probe stays in lock-step with the ONNX Runtime version in Cargo.toml.
//!
//! Probes are best-effort signals, not guarantees: a library that resolves
//! here might still fail deep inside CUDA init (wrong cuDNN version, unsupported
//! compute capability, …). But the converse is solid — **if a probe reports a
//! dep missing, the provider will not work**.

#![allow(unsafe_code)]

use ort::execution_providers::cuda::CUDA_DYLIBS;
use ort::execution_providers::{CUDAExecutionProvider, CoreMLExecutionProvider, ExecutionProvider};

/// Result of probing a single execution provider.
#[derive(Debug, Clone)]
pub struct ProbeResult {
    /// Provider name that matches our own labels ("Cuda", "CoreMl", "Cpu").
    pub provider: &'static str,
    /// Whether every required dependency resolved via the dynamic loader.
    pub all_resolved: bool,
    /// Names of dylibs / reasons the probe failed. Empty when `all_resolved`.
    pub missing: Vec<String>,
}

impl ProbeResult {
    fn missing_one(provider: &'static str, reason: impl Into<String>) -> Self {
        Self {
            provider,
            all_resolved: false,
            missing: vec![reason.into()],
        }
    }
}

/// Platform-specific soname of the main ONNX Runtime dylib. The probe needs
/// to confirm this loads before touching any ORT API — `ort` itself panics
/// inside `OnceLock` init when the dylib is absent.
#[cfg(target_os = "windows")]
const ORT_DYLIB_DEFAULT: &str = "onnxruntime.dll";
#[cfg(target_vendor = "apple")]
const ORT_DYLIB_DEFAULT: &str = "libonnxruntime.dylib";
#[cfg(all(not(target_os = "windows"), not(target_vendor = "apple")))]
const ORT_DYLIB_DEFAULT: &str = "libonnxruntime.so";

/// Probe whether the ORT dylib itself is loadable. When the app runs through
/// Tauri we set `ORT_DYLIB_PATH` explicitly; in tests / CLI usage we fall back
/// to the platform-default soname and let the dynamic loader search.
fn ort_dylib_loadable() -> bool {
    let path = std::env::var("ORT_DYLIB_PATH").unwrap_or_else(|_| ORT_DYLIB_DEFAULT.to_string());
    // Safety: same contract as below — name is a controlled constant or the
    // operator-supplied env path; no symbols are called.
    unsafe { libloading::Library::new(&path) }.is_ok()
}

/// Probe the CUDA execution provider. Returns `all_resolved=true` only when
/// (a) the ORT dylib loads, (b) ONNX Runtime's provider list includes CUDA,
/// and (c) every CUDA runtime dylib named in [`CUDA_DYLIBS`] can be
/// `dlopen`ed from the current process.
pub fn probe_cuda() -> ProbeResult {
    if !ort_dylib_loadable() {
        return ProbeResult::missing_one(
            "Cuda",
            format!("ORT dylib not loadable ({ORT_DYLIB_DEFAULT} or $ORT_DYLIB_PATH)"),
        );
    }

    match CUDAExecutionProvider::default().is_available() {
        Ok(false) => {
            return ProbeResult::missing_one(
                "Cuda",
                "ORT provider list has no CUDA entry (provider dylib missing or CPU-only build)",
            );
        }
        Err(e) => {
            return ProbeResult::missing_one(
                "Cuda",
                format!("ORT GetAvailableProviders failed: {e}"),
            );
        }
        Ok(true) => {}
    }

    let mut missing = Vec::new();
    for name in CUDA_DYLIBS {
        // Safety: we dlopen a well-known soname and drop the handle at scope
        // exit. No foreign symbols are called — this only exercises the
        // loader's resolution step. Per Rust's convention, `Library::new` is
        // still marked unsafe because loading arbitrary libraries can run
        // C++ static initializers; here the names are compile-time pinned.
        let loaded = unsafe { libloading::Library::new(*name) };
        if loaded.is_err() {
            missing.push((*name).to_string());
        }
    }

    ProbeResult {
        provider: "Cuda",
        all_resolved: missing.is_empty(),
        missing,
    }
}

/// Probe the CoreML execution provider. Apple ships the framework with the
/// OS, so on Apple platforms this only needs ORT to know the provider exists;
/// everywhere else we short-circuit to unsupported.
pub fn probe_coreml() -> ProbeResult {
    if !cfg!(target_vendor = "apple") {
        return ProbeResult::missing_one("CoreMl", "CoreML is only available on Apple platforms");
    }
    if !ort_dylib_loadable() {
        return ProbeResult::missing_one(
            "CoreMl",
            format!("ORT dylib not loadable ({ORT_DYLIB_DEFAULT} or $ORT_DYLIB_PATH)"),
        );
    }
    match CoreMLExecutionProvider::default().is_available() {
        Ok(true) => ProbeResult {
            provider: "CoreMl",
            all_resolved: true,
            missing: Vec::new(),
        },
        Ok(false) => ProbeResult::missing_one("CoreMl", "ORT provider list has no CoreML entry"),
        Err(e) => {
            ProbeResult::missing_one("CoreMl", format!("ORT GetAvailableProviders failed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuda_probe_returns_cuda_label() {
        let r = probe_cuda();
        assert_eq!(r.provider, "Cuda");
        // Liveness only — the actual `all_resolved` outcome depends on the
        // host machine. On CI without CUDA it's false; on the dev box with
        // libnvrtc etc. in LD_LIBRARY_PATH it's true.
        assert_eq!(r.all_resolved, r.missing.is_empty());
    }

    #[test]
    fn coreml_probe_only_resolves_on_apple() {
        let r = probe_coreml();
        assert_eq!(r.provider, "CoreMl");
        if !cfg!(target_vendor = "apple") {
            assert!(!r.all_resolved);
            assert!(!r.missing.is_empty());
        }
    }
}
