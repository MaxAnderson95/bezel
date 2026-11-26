//! Remote audio on the Mac: IronRDP's playback on the default output device, silent while the
//! Session is muted. IronRDP's own sound channel has no way to mute one Session, so the engine
//! registers this one in its place.

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ironrdp_rdpsnd::client::{Rdpsnd, RdpsndClientHandler};
use ironrdp_rdpsnd::pdu::{AudioFormat, AudioFormatFlags, PitchPdu, VolumePdu};
use ironrdp_rdpsnd_native::cpal::RdpsndBackend;

/// The sound channel for one connection, playing unless `muted`.
pub(crate) fn channel(muted: Arc<AtomicBool>) -> Rdpsnd {
    let origin = Instant::now();
    Rdpsnd::new(Box::new(Speaker::new(RdpsndBackend::new(), muted))).with_clock(Box::new(move || origin.elapsed()))
}

#[derive(Debug)]
struct Speaker<H> {
    inner: H,
    muted: Arc<AtomicBool>,
}

impl<H: RdpsndClientHandler> Speaker<H> {
    fn new(inner: H, muted: Arc<AtomicBool>) -> Self {
        Self { inner, muted }
    }
}

impl<H: RdpsndClientHandler> RdpsndClientHandler for Speaker<H> {
    fn get_flags(&self) -> AudioFormatFlags {
        self.inner.get_flags()
    }

    fn get_formats(&self) -> &[AudioFormat] {
        self.inner.get_formats()
    }

    fn choose_formats(&self, server_formats: &[AudioFormat]) -> Vec<AudioFormat> {
        self.inner.choose_formats(server_formats)
    }

    /// Muted, the wave is dropped; the channel still confirms it, so Windows plays on.
    fn wave(&mut self, format: &AudioFormat, ts: u32, data: Cow<'_, [u8]>) {
        if !self.muted.load(Ordering::Relaxed) {
            self.inner.wave(format, ts, data);
        }
    }

    fn playback_latency(&self) -> Duration {
        self.inner.playback_latency()
    }

    fn set_volume(&mut self, volume: VolumePdu) {
        self.inner.set_volume(volume);
    }

    fn set_pitch(&mut self, pitch: PitchPdu) {
        self.inner.set_pitch(pitch);
    }

    fn close(&mut self) {
        self.inner.close();
    }
}

#[cfg(test)]
mod tests {
    use ironrdp_rdpsnd::pdu::WaveFormat;

    use super::*;

    /// Counts the waves it is given.
    #[derive(Debug, Default)]
    struct Count(usize);

    impl RdpsndClientHandler for Count {
        fn get_formats(&self) -> &[AudioFormat] {
            &[]
        }

        fn wave(&mut self, _: &AudioFormat, _: u32, _: Cow<'_, [u8]>) {
            self.0 += 1;
        }

        fn set_volume(&mut self, _: VolumePdu) {}

        fn set_pitch(&mut self, _: PitchPdu) {}

        fn close(&mut self) {}
    }

    #[test]
    fn muted_waves_never_reach_the_speaker() {
        let muted = Arc::new(AtomicBool::new(false));
        let mut speaker = Speaker::new(Count::default(), muted.clone());
        let format = AudioFormat {
            format: WaveFormat::PCM,
            n_channels: 2,
            n_samples_per_sec: 44100,
            n_avg_bytes_per_sec: 176_400,
            n_block_align: 4,
            bits_per_sample: 16,
            data: None,
        };
        let play = |speaker: &mut Speaker<Count>| speaker.wave(&format, 0, Cow::Borrowed(&[0; 4]));
        play(&mut speaker);
        muted.store(true, Ordering::Relaxed);
        play(&mut speaker);
        muted.store(false, Ordering::Relaxed);
        play(&mut speaker);
        assert_eq!(speaker.inner.0, 2);
    }
}
