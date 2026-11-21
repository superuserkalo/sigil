//! Every pipeline the viewer builds, created on this platform's real backend: on Windows that is
//! D3D12 with FXC (shader model 5.1), the compiler that rejected the 0.1.20 scene shader on
//! players' machines. Both the default adapter and the software one (WARP on Windows, the
//! viewer's fallback) must build them; any validation, internal or out-of-memory error fails.
//!
//! Without an adapter it skips, unless `SIGIL_REQUIRE_GPU` is set: the Windows workflow
//! (`.github/workflows/windows-shaders.yml`) sets it, so a runner without D3D12 fails loudly.

use sigil_viewer::gpu::{self, Gpu, Pipelines};
use sigil_viewer::particles::{ParticleGpu, fixture};

/// The particle spritecard pipelines both bundles' renderers ask for, built inside error scopes.
fn particle_pipelines(gpu: &Gpu, pipes: &Pipelines) -> Result<(), String> {
    let scopes = [gpu.device.push_error_scope(wgpu::ErrorFilter::Validation), gpu.device.push_error_scope(wgpu::ErrorFilter::Internal)];
    for bundle in [fixture::hero_bundle(), fixture::hideout_bundle()] {
        let _ = ParticleGpu::new(&gpu.device, &gpu.queue, pipes.frame_layout(), gpu.msaa, gpu::HDR, gpu::DEPTH, &bundle, u64::MAX);
    }
    for scope in scopes.into_iter().rev() {
        if let Some(e) = pollster::block_on(scope.pop()) {
            return Err(e.to_string());
        }
    }
    Ok(())
}

#[test]
fn every_pipeline_builds_on_the_default_and_the_software_adapter() {
    let require = std::env::var_os("SIGIL_REQUIRE_GPU").is_some();
    let mut built = Vec::new();
    for (which, gpu) in [("default", Gpu::new()), ("software", Gpu::software())] {
        let gpu = match gpu {
            Ok(gpu) => gpu,
            Err(e) => {
                eprintln!("{which} adapter: {e}; skipped");
                continue;
            }
        };
        let name = format!("{} ({:?}, {:?})", gpu.info.name, gpu.info.backend, gpu.info.device_type);
        eprintln!("{which} adapter: {name}");
        let pipes = Pipelines::new(&gpu).unwrap_or_else(|e| panic!("{which} adapter {name}: the viewer's pipelines failed: {e}"));
        particle_pipelines(&gpu, &pipes).unwrap_or_else(|e| panic!("{which} adapter {name}: the particle pipelines failed: {e}"));
        if let Some(e) = gpu.take_error() {
            panic!("{which} adapter {name}: {e}");
        }
        built.push(which);
    }
    if require {
        assert!(built.contains(&"default"), "SIGIL_REQUIRE_GPU is set but no adapter opened");
        if cfg!(windows) {
            // The viewer falls back to WARP when the driver fails; it must build there too.
            assert!(built.contains(&"software"), "SIGIL_REQUIRE_GPU is set but WARP did not open");
        }
    }
}
