//! Microphones through `cpal` (feature `record`): the input list and the
//! capture source.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    BufferSize, Device, ErrorKind, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedStreamConfig, SupportedStreamConfigRange,
};
use freecut_timeline::time::SampleRate;

use super::source::{CaptureSink, CaptureSource, InputSample, SourceFailure, SourceFormat};
use super::{Channels, RecordError};

/// Rates offered in the device list and tried in order after the
/// preferred one, when the device supports them.
const COMMON_RATES: [u32; 11] = [
    48_000, 44_100, 96_000, 88_200, 32_000, 24_000, 22_050, 16_000, 11_025, 8_000, 192_000,
];

/// An input device, for the recorder's device list (AU-027).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputDeviceInfo {
    /// Stable identifier to open it with ([`CpalInput::open`]).
    pub id: String,
    /// The name to show.
    pub name: String,
    pub is_default: bool,
    /// Sample rates FreeCut can record from it, ascending.
    pub rates: Vec<u32>,
    pub max_channels: u16,
}

/// The input devices the system offers, the default first-class marked.
/// Devices that can't describe themselves (being unplugged) are skipped.
///
/// Asks the audio server and may block for a while: call it off the UI
/// thread.
pub fn list_inputs() -> Result<Vec<InputDeviceInfo>, RecordError> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| device_id(&d));
    let devices = host.input_devices().map_err(device_error)?;
    let mut list: Vec<InputDeviceInfo> = Vec::new();
    for device in devices {
        let (Some(id), Some(name)) = (device_id(&device), device_name(&device)) else {
            continue;
        };
        if list.iter().any(|d| d.id == id) {
            continue;
        }
        let ranges: Vec<SupportedStreamConfigRange> = match device.supported_input_configs() {
            Ok(r) => r.filter(|r| handled(r.sample_format())).collect(),
            Err(_) => continue,
        };
        if ranges.is_empty() {
            continue;
        }
        let mut rates: Vec<u32> = COMMON_RATES
            .into_iter()
            .filter(|&hz| SampleRate::new(hz).is_ok())
            .filter(|&hz| {
                ranges
                    .iter()
                    .any(|r| r.min_sample_rate() <= hz && hz <= r.max_sample_rate())
            })
            .collect();
        rates.sort_unstable();
        list.push(InputDeviceInfo {
            is_default: default.as_deref() == Some(id.as_str()),
            max_channels: ranges.iter().map(|r| r.channels()).max().unwrap_or(0),
            id,
            name,
            rates,
        });
    }
    Ok(list)
}

fn device_id(device: &Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

fn device_name(device: &Device) -> Option<String> {
    device.description().ok().map(|d| d.name().to_owned())
}

fn device_error(e: cpal::Error) -> RecordError {
    match e.kind() {
        ErrorKind::DeviceNotAvailable | ErrorKind::HostUnavailable => RecordError::NoDevice,
        ErrorKind::DeviceBusy => RecordError::Busy(e.to_string()),
        ErrorKind::PermissionDenied => RecordError::Denied(e.to_string()),
        ErrorKind::UnsupportedConfig => RecordError::UnsupportedFormat(e.to_string()),
        _ => RecordError::Device(e.to_string()),
    }
}

/// Sample formats the sink converts, in the order a choice prefers them.
const HANDLED_FORMATS: [SampleFormat; 7] = [
    SampleFormat::F32,
    SampleFormat::I32,
    SampleFormat::I16,
    SampleFormat::F64,
    SampleFormat::U16,
    SampleFormat::I8,
    SampleFormat::U8,
];

fn handled(format: SampleFormat) -> bool {
    HANDLED_FORMATS.contains(&format)
}

/// The configuration to capture with: the preferred rate if the device
/// offers it, else the device's own rate when it is on the tick grid, else
/// the common rates in order; at that rate, the take's channel count when
/// offered (else the fewest channels above it, else any), in the preferred
/// sample format.
fn pick_config(
    default: Option<&SupportedStreamConfig>,
    ranges: &[SupportedStreamConfigRange],
    preferred_rate: u32,
    channels: u16,
) -> Result<SupportedStreamConfig, RecordError> {
    let own = default.map(SupportedStreamConfig::sample_rate);
    let rates = std::iter::once(preferred_rate)
        .chain(own)
        .chain(COMMON_RATES)
        .filter(|&hz| SampleRate::new(hz).is_ok());
    for hz in rates {
        let mut fits: Vec<&SupportedStreamConfigRange> = ranges
            .iter()
            .filter(|r| handled(r.sample_format()))
            .filter(|r| r.min_sample_rate() <= hz && hz <= r.max_sample_rate())
            .collect();
        if fits.is_empty() {
            continue;
        }
        // Exact channel count first, then the smallest that covers it,
        // then the largest; then the preferred format.
        let rank = |r: &SupportedStreamConfigRange| {
            let c = r.channels();
            let fit = if c == channels {
                0
            } else if c > channels {
                1
            } else {
                2
            };
            let distance = c.abs_diff(channels);
            let format = HANDLED_FORMATS
                .iter()
                .position(|f| *f == r.sample_format())
                .unwrap_or(HANDLED_FORMATS.len());
            (fit, distance, format)
        };
        fits.sort_by_key(|r| rank(r));
        if let Some(best) = fits.first()
            && let Some(config) = (**best).try_with_sample_rate(hz)
        {
            return Ok(config);
        }
    }
    match default {
        Some(d) if !handled(d.sample_format()) => Err(RecordError::UnsupportedFormat(
            d.sample_format().to_string(),
        )),
        Some(d) => Err(RecordError::UnsupportedRate(d.sample_rate())),
        None => Err(RecordError::UnsupportedFormat(
            "no input configuration".to_owned(),
        )),
    }
}

/// A microphone as a [`CaptureSource`]. Opening it picks the configuration;
/// [`CaptureSource::start`] builds and plays the stream.
pub struct CpalInput {
    device: Device,
    name: String,
    config: StreamConfig,
    format: SampleFormat,
    stream: Option<Stream>,
    latency: Arc<AtomicU64>,
}

impl std::fmt::Debug for CpalInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpalInput")
            .field("name", &self.name)
            .field("rate", &self.config.sample_rate)
            .field("channels", &self.config.channels)
            .field("format", &self.format)
            .field("running", &self.stream.is_some())
            .finish()
    }
}

impl CpalInput {
    /// Opens the input `id` (from [`list_inputs`]; `None`: the system
    /// default) for a take of `channels`, at `preferred_rate` (the
    /// project's) when the device supports it, else at its own rate.
    ///
    /// May block while the system answers: call it off the UI thread.
    pub fn open(
        id: Option<&str>,
        preferred_rate: u32,
        channels: Channels,
    ) -> Result<CpalInput, RecordError> {
        let host = cpal::default_host();
        let device = match id {
            None => host.default_input_device().ok_or(RecordError::NoDevice)?,
            Some(id) => host
                .input_devices()
                .map_err(device_error)?
                .find(|d| device_id(d).as_deref() == Some(id))
                .ok_or_else(|| RecordError::DeviceNotFound(id.to_owned()))?,
        };
        let name = device_name(&device).ok_or(RecordError::NoDevice)?;
        let default = device.default_input_config().ok();
        let ranges: Vec<SupportedStreamConfigRange> = device
            .supported_input_configs()
            .map_err(device_error)?
            .collect();
        let supported = pick_config(default.as_ref(), &ranges, preferred_rate, channels.count())?;
        let format = supported.sample_format();
        let mut config = supported.config();
        config.buffer_size = BufferSize::Default;
        Ok(CpalInput {
            device,
            name,
            config,
            format,
            stream: None,
            latency: Arc::default(),
        })
    }

    /// The device's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    fn build<T: SizedSample + InputSample>(
        &self,
        mut sink: CaptureSink,
    ) -> Result<Stream, cpal::Error> {
        let fail = sink.fail_handle();
        let latency = Arc::clone(&self.latency);
        self.device.build_input_stream::<T, _, _>(
            self.config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                let ts = info.timestamp();
                let delay = ts
                    .callback
                    .checked_duration_since(ts.capture)
                    .unwrap_or_default();
                latency.store(
                    u64::try_from(delay.as_nanos()).unwrap_or(u64::MAX),
                    Ordering::Relaxed,
                );
                let now = Instant::now();
                sink.push(data, now.checked_sub(delay).unwrap_or(now));
            },
            move |err: cpal::Error| match err.kind() {
                ErrorKind::DeviceNotAvailable => fail.fail(SourceFailure::DeviceLost),
                // The configuration no longer holds (a rate or format change).
                ErrorKind::StreamInvalidated => fail.fail(SourceFailure::RateChanged),
                ErrorKind::Xrun => tracing::warn!("audio input lost samples: {err}"),
                _ => tracing::warn!("audio input error: {err}"),
            },
            None,
        )
    }
}

impl CaptureSource for CpalInput {
    fn format(&self) -> SourceFormat {
        SourceFormat {
            rate: self.config.sample_rate,
            channels: self.config.channels,
        }
    }

    fn start(&mut self, sink: CaptureSink) -> Result<(), RecordError> {
        self.stop();
        let stream = match self.format {
            SampleFormat::F32 => self.build::<f32>(sink),
            SampleFormat::I32 => self.build::<i32>(sink),
            SampleFormat::I16 => self.build::<i16>(sink),
            SampleFormat::F64 => self.build::<f64>(sink),
            SampleFormat::U16 => self.build::<u16>(sink),
            SampleFormat::I8 => self.build::<i8>(sink),
            SampleFormat::U8 => self.build::<u8>(sink),
            other => return Err(RecordError::UnsupportedFormat(other.to_string())),
        }
        .map_err(device_error)?;
        stream.play().map_err(device_error)?;
        tracing::info!(device = %self.name, rate = self.config.sample_rate, "audio input started");
        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) {
        // Dropping the stream stops its callback.
        self.stream = None;
    }

    fn latency(&self) -> Duration {
        Duration::from_nanos(self.latency.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use cpal::SupportedBufferSize;

    use super::*;

    fn range(
        channels: u16,
        format: SampleFormat,
        min: u32,
        max: u32,
    ) -> SupportedStreamConfigRange {
        SupportedStreamConfigRange::new(channels, min, max, SupportedBufferSize::Unknown, format)
    }

    fn default_config(channels: u16, format: SampleFormat, hz: u32) -> SupportedStreamConfig {
        SupportedStreamConfig::new(channels, hz, SupportedBufferSize::Unknown, format)
    }

    #[test]
    fn the_project_rate_and_take_channels_are_preferred() {
        let ranges = [
            range(2, SampleFormat::F32, 44_100, 48_000),
            range(1, SampleFormat::I16, 44_100, 48_000),
            range(1, SampleFormat::F32, 44_100, 48_000),
        ];
        let d = default_config(2, SampleFormat::F32, 44_100);
        let c = pick_config(Some(&d), &ranges, 48_000, 1).unwrap();
        assert_eq!(
            (c.sample_rate(), c.channels(), c.sample_format()),
            (48_000, 1, SampleFormat::F32)
        );
        let c = pick_config(Some(&d), &ranges, 48_000, 2).unwrap();
        assert_eq!((c.sample_rate(), c.channels()), (48_000, 2));
    }

    #[test]
    fn an_unsupported_project_rate_falls_back_to_the_device_rate() {
        // A USB microphone fixed at 44.1 kHz with four channels.
        let ranges = [
            range(4, SampleFormat::I24, 44_100, 44_100),
            range(4, SampleFormat::I16, 44_100, 44_100),
        ];
        let d = default_config(4, SampleFormat::I24, 44_100);
        let c = pick_config(Some(&d), &ranges, 48_000, 2).unwrap();
        assert_eq!(
            (c.sample_rate(), c.channels(), c.sample_format()),
            (44_100, 4, SampleFormat::I16)
        );
        let only_24 = [range(2, SampleFormat::I24, 48_000, 48_000)];
        let d = default_config(2, SampleFormat::I24, 48_000);
        assert!(matches!(
            pick_config(Some(&d), &only_24, 48_000, 1),
            Err(RecordError::UnsupportedFormat(_))
        ));
    }

    /// Records from the real default microphone. Needs hardware and
    /// permission, so it only runs on request.
    #[test]
    #[ignore = "needs a microphone"]
    fn lists_the_inputs_and_opens_the_default() {
        let inputs = list_inputs().unwrap();
        assert!(!inputs.is_empty());
        let input = CpalInput::open(None, 48_000, Channels::Mono).unwrap();
        assert!(input.format().rate > 0);
    }
}
