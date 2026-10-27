//! Adapter capabilities and headless devices.

/// What the adapter can do, queried once at startup.
#[derive(Clone, Debug)]
pub struct GpuCapabilities {
    pub info: wgpu::AdapterInfo,
    pub limits: wgpu::Limits,
    pub downlevel: wgpu::DownlevelCapabilities,
    /// 4 when the Canvas format supports 4x MSAA with resolve, otherwise 1.
    pub msaa_samples: u32,
}

/// The smallest `max_texture_dimension_2d` the app supports: the atlas page size floor.
const MIN_TEXTURE_DIMENSION: u32 = 2048;

/// Whether the adapter is good enough, decided once at startup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Suitability {
    Ok,
    /// A CPU adapter (lavapipe, WARP): usable, but the app may be slow. Carries the message.
    SoftwareWarning(String),
    /// Below the hardware floor. Carries the error message.
    BelowFloor(String),
}

/// "name (max_texture_dimension_2d N, ...)" for error messages.
pub(crate) fn describe_limits(name: &str, limits: &wgpu::Limits) -> String {
    format!(
        "{name} (max_texture_dimension_2d {}, max_buffer_size {}, max_texture_array_layers {})",
        limits.max_texture_dimension_2d, limits.max_buffer_size, limits.max_texture_array_layers
    )
}

impl GpuCapabilities {
    /// Applies the hardware floor (any adapter with 2D textures of at least
    /// [`MIN_TEXTURE_DIMENSION`]) and flags software adapters.
    pub fn suitability(&self) -> Suitability {
        let name = &self.info.name;
        let limit = self.limits.max_texture_dimension_2d;
        if limit < MIN_TEXTURE_DIMENSION {
            return Suitability::BelowFloor(format!(
                "The GPU \"{name}\" is too limited: max_texture_dimension_2d is {limit}, \
                 and at least {MIN_TEXTURE_DIMENSION} is required."
            ));
        }
        if self.info.device_type == wgpu::DeviceType::Cpu {
            return Suitability::SoftwareWarning(format!(
                "The GPU \"{name}\" is a software adapter, so the app may be slow."
            ));
        }
        Suitability::Ok
    }

    pub fn query(adapter: &wgpu::Adapter) -> Self {
        let flags = adapter
            .get_texture_format_features(crate::CANVAS_FORMAT)
            .flags;
        let msaa = flags.sample_count_supported(4)
            && flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE);
        Self {
            info: adapter.get_info(),
            limits: adapter.limits(),
            downlevel: adapter.get_downlevel_capabilities(),
            msaa_samples: if msaa { 4 } else { 1 },
        }
    }
}

/// A device with no window, for export and tests.
pub struct Headless {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

#[derive(Debug, thiserror::Error)]
pub enum HeadlessError {
    #[error("no GPU adapter: {0}")]
    NoAdapter(#[source] wgpu::RequestAdapterError),
    #[error("could not open the GPU device: {0}")]
    Device(#[source] wgpu::RequestDeviceError),
}

impl Headless {
    /// Opens a device. `software` asks for the platform's software adapter (lavapipe on
    /// Linux, WARP on Windows), which golden tests use for reproducible pixels.
    pub fn new(software: bool) -> Result<Self, HeadlessError> {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(
                wgpu::InstanceDescriptor::new_without_display_handle_from_env(),
            );
            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    force_fallback_adapter: software,
                    compatible_surface: None,
                    apply_limit_buckets: false,
                })
                .await
                .map_err(HeadlessError::NoAdapter)?;
            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("headless"),
                    required_limits: wgpu::Limits::downlevel_defaults()
                        .using_resolution(adapter.limits()),
                    ..Default::default()
                })
                .await
                .map_err(HeadlessError::Device)?;
            Ok(Self {
                adapter,
                device,
                queue,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(device_type: wgpu::DeviceType, max_texture: u32) -> GpuCapabilities {
        let mut info = wgpu::AdapterInfo::new(wgpu::DeviceType::Cpu, wgpu::Backend::Vulkan);
        info.name = "test adapter".into();
        info.device_type = device_type;
        GpuCapabilities {
            info,
            limits: wgpu::Limits {
                max_texture_dimension_2d: max_texture,
                ..wgpu::Limits::downlevel_defaults()
            },
            downlevel: wgpu::DownlevelCapabilities::default(),
            msaa_samples: 1,
        }
    }

    #[test]
    fn hardware_at_the_floor_is_ok() {
        let caps = capabilities(wgpu::DeviceType::IntegratedGpu, 2048);
        assert_eq!(caps.suitability(), Suitability::Ok);
    }

    #[test]
    fn software_adapter_warns() {
        let caps = capabilities(wgpu::DeviceType::Cpu, 8192);
        let Suitability::SoftwareWarning(message) = caps.suitability() else {
            panic!("expected a warning");
        };
        assert!(message.contains("test adapter") && message.contains("slow"));
    }

    #[test]
    fn below_the_floor_names_adapter_and_limit() {
        let caps = capabilities(wgpu::DeviceType::DiscreteGpu, 1024);
        let Suitability::BelowFloor(message) = caps.suitability() else {
            panic!("expected an error");
        };
        assert!(message.contains("test adapter"));
        assert!(message.contains("max_texture_dimension_2d is 1024"));
    }

    #[test]
    fn too_small_wins_over_software() {
        let caps = capabilities(wgpu::DeviceType::Cpu, 1024);
        assert!(matches!(caps.suitability(), Suitability::BelowFloor(_)));
    }
}
