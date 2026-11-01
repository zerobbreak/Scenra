//! The presenter's mic, captured with WASAPI (shared mode, the Windows default input device) and
//! appended as raw 16-bit PCM to a `.part` file; `wav::finish` turns it into `microphone.wav`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;
use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

pub const SAMPLE_RATE: u32 = 48_000;
pub const CHANNELS: u16 = 1;

pub struct Mic {
    pub device: String,
    part: PathBuf,
    stop: Arc<AtomicBool>,
    first_qpc: Arc<AtomicI64>,
    thread: JoinHandle<Result<(), String>>,
}

pub struct MicStats {
    pub part: PathBuf,
    /// QPC (100 ns) of the first captured sample.
    pub first_qpc: i64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// The name of the Windows default recording device, if there is one.
pub fn default_device_name() -> Option<String> {
    let _ = wasapi::initialize_mta();
    DeviceEnumerator::new().ok()?.get_default_device(&Direction::Capture).ok()?.get_friendlyname().ok()
}

impl Mic {
    /// Opens the default mic and starts writing to `part`. Returns once the stream is running.
    pub fn start(part: &Path) -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let first_qpc = Arc::new(AtomicI64::new(0));
        let (ready_tx, ready_rx) = mpsc::channel();
        let thread = {
            let (part, stop, first_qpc) = (part.to_owned(), stop.clone(), first_qpc.clone());
            std::thread::Builder::new()
                .name("scenra-mic".into())
                .spawn(move || capture(&part, &stop, &first_qpc, ready_tx))
                .map_err(err)?
        };
        match ready_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(device)) => Ok(Self { device, part: part.to_owned(), stop, first_qpc, thread }),
            Ok(Err(e)) => Err(e),
            Err(_) => {
                stop.store(true, Ordering::SeqCst);
                Err("The microphone didn't start within 5 s".into())
            }
        }
    }

    pub fn first_qpc(&self) -> i64 {
        self.first_qpc.load(Ordering::SeqCst)
    }

    pub fn stop(self) -> Result<MicStats, String> {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.join().map_err(|_| "the microphone thread panicked".to_owned())??;
        Ok(MicStats { part: self.part, first_qpc: self.first_qpc.load(Ordering::SeqCst) })
    }
}

/// Frames of silence to insert before a packet stamped `qpc`, when it starts later than the
/// frames written so far reach (by more than 20 ms, to ignore timestamp jitter).
fn missing_frames(first_qpc: i64, written: u64, qpc: i64) -> u64 {
    let expected = first_qpc + (written as i64) * 10_000_000 / i64::from(SAMPLE_RATE);
    let late = qpc - expected;
    if late <= 200_000 {
        return 0;
    }
    (late as u64) * u64::from(SAMPLE_RATE) / 10_000_000
}

fn capture(
    part: &Path, stop: &AtomicBool, first_qpc: &AtomicI64, ready: mpsc::Sender<Result<String, String>>,
) -> Result<(), String> {
    let opened = (|| -> Result<_, String> {
        let _ = wasapi::initialize_mta();
        let device = DeviceEnumerator::new()
            .and_then(|e| e.get_default_device(&Direction::Capture))
            .map_err(|_| "No microphone found: set a default recording device in Windows Sound settings".to_owned())?;
        let name = device.get_friendlyname().unwrap_or_else(|_| "Microphone".into());
        let mut client = device.get_iaudioclient().map_err(err)?;
        let format = WaveFormat::new(16, 16, &SampleType::Int, SAMPLE_RATE as usize, usize::from(CHANNELS), None);
        let (_, min_period) = client.get_device_period().map_err(err)?;
        let mode = StreamMode::EventsShared { autoconvert: true, buffer_duration_hns: min_period };
        client.initialize_client(&format, &Direction::Capture, &mode).map_err(err)?;
        let event = client.set_get_eventhandle().map_err(err)?;
        let reader = client.get_audiocaptureclient().map_err(err)?;
        let file = BufWriter::new(File::create(part).map_err(|e| format!("{}: {e}", part.display()))?);
        client.start_stream().map_err(err)?;
        Ok((name, client, event, reader, file))
    })();
    let (name, client, event, reader, mut file) = match opened {
        Ok(opened) => opened,
        Err(e) => {
            let _ = ready.send(Err(e.clone()));
            return Err(e);
        }
    };
    let _ = ready.send(Ok(name));

    let block = usize::from(CHANNELS) * 2;
    let mut buf = vec![0u8; SAMPLE_RATE as usize * block];
    let mut written: u64 = 0;
    let result = (|| -> Result<(), String> {
        while !stop.load(Ordering::SeqCst) {
            if event.wait_for_event(200).is_err() {
                continue;
            }
            while let Some(frames) = reader.get_next_packet_size().map_err(err)? {
                if frames == 0 {
                    break;
                }
                let (got, info) = reader.read_from_device(&mut buf).map_err(err)?;
                if got == 0 {
                    break;
                }
                // The packet's QPC position, 100 ns units: the same clock as WGC frame times.
                let _ = first_qpc.compare_exchange(0, info.timestamp as i64, Ordering::SeqCst, Ordering::SeqCst);
                if !info.flags.timestamp_error {
                    // Samples Windows dropped (a glitch on a busy machine) become silence, so
                    // everything after them stays on the video's clock.
                    let gap = missing_frames(first_qpc.load(Ordering::SeqCst), written, info.timestamp as i64);
                    if gap > 0 {
                        file.write_all(&vec![0u8; gap as usize * block]).map_err(err)?;
                        written += gap;
                    }
                }
                let data = &mut buf[..got as usize * block];
                if info.flags.silent {
                    data.fill(0);
                }
                file.write_all(data).map_err(err)?;
                written += u64::from(got);
            }
        }
        Ok(())
    })();
    let _ = client.stop_stream();
    file.flush().map_err(err)?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dropped_samples_are_filled_but_jitter_is_not() {
        let rate = u64::from(SAMPLE_RATE);
        // One second written, next packet on time or 10 ms late: nothing to fill.
        assert_eq!(missing_frames(1_000, rate, 1_000 + 10_000_000), 0);
        assert_eq!(missing_frames(1_000, rate, 1_000 + 10_100_000), 0);
        // 200 ms of samples lost: fill exactly that.
        assert_eq!(missing_frames(1_000, rate, 1_000 + 12_000_000), rate / 5);
    }
}
