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

impl GpuCapabilities {
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

#[derive(Debug)]
pub enum HeadlessError {
    NoAdapter(wgpu::RequestAdapterError),
    Device(wgpu::RequestDeviceError),
}

impl std::fmt::Display for HeadlessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeadlessError::NoAdapter(err) => write!(f, "no GPU adapter: {err}"),
            HeadlessError::Device(err) => write!(f, "could not open the GPU device: {err}"),
        }
    }
}

impl std::error::Error for HeadlessError {}

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
