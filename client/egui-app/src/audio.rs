//! In-process audio engine: HTTP range source, symphonia demux + AAC decode, libopus for Opus,
//! cpal output. Decode runs on a worker thread per load; the output callback drains a bounded ring.
#![allow(dead_code)]

use std::collections::{HashMap, VecDeque};
use std::io::{self, Read, Seek, SeekFrom};
use std::sync::atomic::AtomicBool;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, SupportedStreamConfig};
use symphonia::core::codecs::audio::{AudioDecoder, AudioDecoderOptions};
use symphonia::core::codecs::CodecParameters;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo, TrackType};
use symphonia::core::io::{MediaSource, MediaSourceStream, MediaSourceStreamOptions};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::packet::Packet;
use symphonia::core::units::{Time, TimeBase};
use symphonia::default::codecs::AacDecoder;
use symphonia::default::get_probe;

const DEFAULT_CHUNK: u64 = 1 << 20;
const OPUS_RATE: u32 = 48_000;
/// 120 ms at 48 kHz, the largest Opus frame, per channel.
const OPUS_MAX_FRAME: usize = 5760;
const POLL: Duration = Duration::from_millis(10);

#[derive(serde::Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct StreamInfo {
    pub url: String,
    pub headers: HashMap<String, String>,
    pub mime_type: String,
    pub expires_in_seconds: i64,
    pub client: String,
    pub require_bounded_range: bool,
    pub range_chunk_size_bytes: u64,
    pub use_range_chunks: bool,
    #[serde(default)]
    pub duration_sec: Option<u32>,
}

#[derive(Debug, Clone)]
pub enum Event {
    Ended,
    Error(String),
}

/// Inclusive byte range of the `chunk`-sized window containing `pos`, clipped to the file length.
pub fn chunk_range(pos: u64, chunk: u64, total: Option<u64>) -> (u64, u64) {
    let start = pos / chunk * chunk;
    let mut end = start + chunk - 1;
    if let Some(t) = total {
        end = end.min(t.saturating_sub(1));
    }
    (start, end)
}

/// Byte-range reader over the stream URL. Keeps only the chunk that holds the cursor.
struct HttpSource {
    agent: ureq::Agent,
    info: StreamInfo,
    chunk: u64,
    total: Option<u64>,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
}

impl HttpSource {
    fn open(info: &StreamInfo) -> io::Result<Self> {
        let chunk = match info.range_chunk_size_bytes {
            0 => DEFAULT_CHUNK,
            n => n,
        };
        let mut s = HttpSource {
            agent: ureq::AgentBuilder::new().timeout(Duration::from_secs(30)).build(),
            info: info.clone(),
            chunk,
            total: None,
            pos: 0,
            buf: Vec::new(),
            buf_start: 0,
        };
        s.fetch(0)?;
        Ok(s)
    }

    fn fetch(&mut self, at: u64) -> io::Result<()> {
        let (start, end) = chunk_range(at, self.chunk, self.total);
        let mut req = self
            .agent
            .get(&self.info.url)
            .set("Range", &format!("bytes={start}-{end}"));
        for (k, v) in &self.info.headers {
            req = req.set(k, v);
        }
        let resp = req.call().map_err(|e| io::Error::other(e.to_string()))?;
        if resp.status() != 206 {
            return Err(io::Error::other(format!("unexpected HTTP {}", resp.status())));
        }
        if self.total.is_none() {
            self.total = resp
                .header("Content-Range")
                .and_then(|cr| cr.rsplit('/').next())
                .and_then(|t| t.parse().ok());
        }
        let len = end - start + 1;
        let mut body = Vec::with_capacity(len as usize);
        resp.into_reader().take(len).read_to_end(&mut body)?;
        self.buf = body;
        self.buf_start = start;
        Ok(())
    }
}

impl Read for HttpSource {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() || self.total.is_some_and(|t| self.pos >= t) {
            return Ok(0);
        }
        if self.pos < self.buf_start || self.pos >= self.buf_start + self.buf.len() as u64 {
            self.fetch(self.pos)?;
        }
        let off = (self.pos - self.buf_start) as usize;
        let avail = self.buf.len().saturating_sub(off);
        if avail == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "empty range response"));
        }
        let n = avail.min(out.len());
        out[..n].copy_from_slice(&self.buf[off..off + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let (base, delta) = match from {
            SeekFrom::Start(n) => {
                self.pos = n;
                return Ok(n);
            }
            SeekFrom::Current(d) => (self.pos, d),
            SeekFrom::End(d) => (
                self.total.ok_or_else(|| io::Error::other("unknown stream length"))?,
                d,
            ),
        };
        let np = base
            .checked_add_signed(delta)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "seek before start"))?;
        self.pos = np;
        Ok(np)
    }
}

impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        self.total
    }
}

/// Linear interpolation resampler that also maps the channel count.
/// ponytail: linear resample, swap for rubato if quality complaints.
struct Resample {
    ratio: f64,
    pos: f64,
    buf: Vec<f32>,
    out_ch: usize,
}

impl Resample {
    fn new(src_rate: u32, out_rate: u32, out_ch: usize) -> Self {
        Resample { ratio: src_rate as f64 / out_rate as f64, pos: 0.0, buf: Vec::new(), out_ch }
    }

    fn reset(&mut self) {
        self.buf.clear();
        self.pos = 0.0;
    }

    /// `src` is interleaved with `src_ch` channels; appends interleaved `out_ch` samples to `out`.
    fn process(&mut self, src: &[f32], src_ch: usize, out: &mut Vec<f32>) {
        let ch = self.out_ch;
        for f in src.chunks_exact(src_ch) {
            if ch == 1 {
                self.buf.push(f.iter().sum::<f32>() / src_ch as f32);
            } else {
                self.buf.extend((0..ch).map(|c| f[c.min(src_ch - 1)]));
            }
        }
        let frames = self.buf.len() / ch;
        while (self.pos as usize) + 1 < frames {
            let i = self.pos as usize;
            let t = (self.pos - i as f64) as f32;
            for c in 0..ch {
                let (a, b) = (self.buf[i * ch + c], self.buf[(i + 1) * ch + c]);
                out.push(a + (b - a) * t);
            }
            self.pos += self.ratio;
        }
        let keep = (self.pos as usize).min(frames);
        self.buf.drain(..keep * ch);
        self.pos -= keep as f64;
    }
}

enum Codec {
    Opus(opus::Decoder),
    Aac(Box<dyn AudioDecoder>),
}

impl Codec {
    /// Decodes into `tmp` (interleaved f32). Returns (sample rate, channel count).
    fn decode(&mut self, packet: &Packet, tmp: &mut Vec<f32>) -> Result<(u32, usize), ()> {
        match self {
            Codec::Opus(d) => {
                tmp.clear();
                tmp.resize(OPUS_MAX_FRAME * 2, 0.0);
                let n = d.decode_float(&packet.data, tmp, false).map_err(|_| ())?;
                tmp.truncate(n * 2);
                Ok((OPUS_RATE, 2))
            }
            Codec::Aac(d) => {
                let buf = d.decode(packet).map_err(|_| ())?;
                tmp.clear();
                buf.copy_to_vec_interleaved(tmp);
                let spec = buf.spec();
                Ok((spec.rate(), spec.channels().count()))
            }
        }
    }
}

/// Everything owned by one decode thread: the demuxer, the codec and the current track position.
struct Session {
    reader: Box<dyn FormatReader>,
    track_id: u32,
    time_base: Option<TimeBase>,
    codec: Codec,
    tmp: Vec<f32>,
    resample: Option<Resample>,
    duration: Option<f32>,
}

impl Session {
    fn open(info: &StreamInfo) -> Result<Session, String> {
        let src = HttpSource::open(info).map_err(|e| e.to_string())?;
        let mss = MediaSourceStream::new(Box::new(src), MediaSourceStreamOptions::default());
        let mut hint = Hint::new();
        hint.mime_type(info.mime_type.split(';').next().unwrap_or_default().trim());
        let reader = get_probe()
            .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
            .map_err(|e| e.to_string())?;
        let track = reader.default_track(TrackType::Audio).ok_or("no audio track")?;
        let (track_id, time_base) = (track.id, track.time_base);
        let duration = match (track.duration, time_base) {
            (Some(d), Some(tb)) => tb.calc_duration(d).map(|t| t.as_secs_f64() as f32),
            _ => None,
        };
        let codec = if info.mime_type.contains("opus") {
            let d = opus::Decoder::new(OPUS_RATE, opus::Channels::Stereo).map_err(|e| e.to_string())?;
            Codec::Opus(d)
        } else {
            let Some(CodecParameters::Audio(params)) = track.codec_params.as_ref() else {
                return Err("no audio codec parameters".into());
            };
            let d = AacDecoder::try_new(params, &AudioDecoderOptions::default())
                .map_err(|e| e.to_string())?;
            Codec::Aac(Box::new(d))
        };
        Ok(Session { reader, track_id, time_base, codec, tmp: Vec::new(), resample: None, duration })
    }

    /// Seeks the demuxer and decoder and rebases the ring to the actual landing time.
    /// Returns the ring epoch the decode thread should use from now on.
    fn seek(&mut self, shared: &Shared, secs: f32) -> Result<u64, String> {
        let time = Time::try_from_secs_f64(secs as f64).ok_or("invalid seek position")?;
        let to = SeekTo::Time { time, track_id: Some(self.track_id) };
        let landed = self.reader.seek(SeekMode::Accurate, to).map_err(|e| e.to_string())?;
        match &mut self.codec {
            Codec::Opus(d) => {
                let _ = d.reset_state();
            }
            Codec::Aac(d) => d.reset(),
        }
        if let Some(rs) = self.resample.as_mut() {
            rs.reset();
        }
        let base = match self.time_base {
            Some(tb) => tb.calc_time_saturating(landed.actual_ts).as_secs_f64(),
            None => secs as f64,
        };
        Ok(shared.ring.lock().unwrap().rebase(base))
    }

    /// Reads and decodes packets until one yields samples into `pending`. Ok(false) at end of stream.
    fn decode_next(&mut self, shared: &Shared, pending: &mut Vec<f32>) -> Result<bool, String> {
        loop {
            let Some(packet) = self.reader.next_packet().map_err(|e| e.to_string())? else {
                return Ok(false);
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let Ok((rate, ch)) = self.codec.decode(&packet, &mut self.tmp) else {
                continue;
            };
            let rs = self
                .resample
                .get_or_insert_with(|| Resample::new(rate, shared.out_rate, shared.out_ch));
            rs.process(&self.tmp, ch, pending);
            return Ok(true);
        }
    }
}

struct Ring {
    samples: VecDeque<f32>,
    cap: usize,
    /// Output frames consumed by the cpal callback since `base`.
    played: u64,
    /// Track time (s) at which `played` was 0.
    base: f64,
    duration: Option<f32>,
    /// Bumped on load/seek/stop. Decode threads drop samples produced under an older epoch.
    epoch: u64,
}

impl Ring {
    fn rebase(&mut self, base: f64) -> u64 {
        self.samples.clear();
        self.played = 0;
        self.base = base;
        self.epoch
    }

    fn restart(&mut self, base: f64) -> u64 {
        self.epoch += 1;
        self.rebase(base)
    }
}

struct Shared {
    ring: Mutex<Ring>,
    events: Mutex<Vec<Event>>,
    playing: AtomicBool,
    volume: AtomicU32,
    out_rate: u32,
    out_ch: usize,
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn emit(&self, ev: Event) {
        self.events.lock().unwrap().push(ev);
        (self.wake)();
    }

    /// Moves `pending` into the ring once there is room. False means full; retry later.
    fn try_push(&self, epoch: u64, pending: &mut Vec<f32>) -> bool {
        let mut r = self.ring.lock().unwrap();
        if r.epoch != epoch {
            pending.clear();
            return true;
        }
        if !r.samples.is_empty() && r.samples.len() + pending.len() > r.cap {
            return false;
        }
        r.samples.extend(pending.drain(..));
        true
    }

    fn drained(&self, epoch: u64) -> bool {
        let r = self.ring.lock().unwrap();
        r.epoch != epoch || r.samples.is_empty()
    }

    fn finish(&self, epoch: u64) {
        if self.ring.lock().unwrap().epoch != epoch {
            return;
        }
        self.playing.store(false, Relaxed);
        self.emit(Event::Ended);
    }

    fn fail(&self, epoch: u64, msg: String) {
        if self.ring.lock().unwrap().epoch != epoch {
            return;
        }
        self.playing.store(false, Relaxed);
        self.emit(Event::Error(msg));
    }
}

fn run(shared: &Shared, rx: &Receiver<f32>, info: &StreamInfo, mut epoch: u64) -> Result<(), String> {
    let mut s = Session::open(info)?;
    {
        let mut r = shared.ring.lock().unwrap();
        if r.epoch == epoch {
            r.duration = s.duration;
        }
    }
    let mut pending = Vec::new();
    let (mut eos, mut ended) = (false, false);
    loop {
        let cmd = match rx.try_recv() {
            Ok(secs) => Some(secs),
            Err(TryRecvError::Disconnected) => return Ok(()),
            Err(TryRecvError::Empty) if ended => match rx.recv() {
                Ok(secs) => Some(secs),
                Err(_) => return Ok(()),
            },
            Err(TryRecvError::Empty) => None,
        };
        if let Some(secs) = cmd {
            epoch = s.seek(shared, secs)?;
            pending.clear();
            eos = false;
            ended = false;
        } else if !pending.is_empty() {
            if !shared.try_push(epoch, &mut pending) {
                thread::sleep(POLL);
            }
        } else if eos {
            if shared.drained(epoch) {
                shared.finish(epoch);
                ended = true;
            } else {
                thread::sleep(POLL);
            }
        } else if !s.decode_next(shared, &mut pending)? {
            eos = true;
        }
    }
}

fn build_stream(dev: &cpal::Device, cfg: &SupportedStreamConfig, shared: Arc<Shared>) -> Result<cpal::Stream, String> {
    let c = cfg.config();
    let stream = match cfg.sample_format() {
        SampleFormat::F32 => build::<f32>(dev, c, shared)?,
        SampleFormat::I16 => build::<i16>(dev, c, shared)?,
        SampleFormat::U16 => build::<u16>(dev, c, shared)?,
        f => return Err(format!("unsupported sample format {f:?}")),
    };
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build<T: SizedSample + FromSample<f32>>(
    dev: &cpal::Device,
    cfg: cpal::StreamConfig,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    dev.build_output_stream(
        cfg,
        move |data: &mut [T], _| fill(&shared, data),
        |e| eprintln!("audio output: {e}"),
        None,
    )
    .map_err(|e| e.to_string())
}

fn fill<T: FromSample<f32>>(shared: &Shared, data: &mut [T]) {
    let vol = f32::from_bits(shared.volume.load(Relaxed));
    let playing = shared.playing.load(Relaxed);
    let mut r = shared.ring.lock().unwrap();
    let mut got = 0usize;
    for s in data.iter_mut() {
        let next = if playing { r.samples.pop_front() } else { None };
        let v = match next {
            Some(v) => {
                got += 1;
                v * vol
            }
            None => 0.0,
        };
        *s = T::from_sample_(v);
    }
    r.played += (got / shared.out_ch) as u64;
}

fn output_device() -> Option<(cpal::Device, SupportedStreamConfig)> {
    let dev = cpal::default_host().default_output_device()?;
    let cfg = dev
        .supported_output_configs()
        .ok()?
        .filter_map(|r| r.try_with_sample_rate(OPUS_RATE))
        // PipeWire's ALSA plug-in lists every channel count up to 64; the max of an F32-only key picked the last of them.
        .max_by_key(|c| (c.channels() == 2, c.sample_format() == SampleFormat::F32))
        .or_else(|| dev.default_output_config().ok())?;
    Some((dev, cfg))
}

pub struct Engine {
    shared: Arc<Shared>,
    stream: Option<cpal::Stream>,
    tx: Mutex<Option<Sender<f32>>>,
}

impl Engine {
    /// `wake` is called (from any thread) whenever state changes or an event is queued, so the UI can repaint.
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Engine {
        let device = output_device();
        let (out_rate, out_ch) = match &device {
            Some((_, c)) => (c.sample_rate(), c.channels() as usize),
            None => (OPUS_RATE, 2),
        };
        let shared = Arc::new(Shared {
            ring: Mutex::new(Ring {
                samples: VecDeque::new(),
                cap: out_rate as usize * out_ch * 2,
                played: 0,
                base: 0.0,
                duration: None,
                epoch: 0,
            }),
            events: Mutex::new(Vec::new()),
            playing: AtomicBool::new(false),
            volume: AtomicU32::new(1f32.to_bits()),
            out_rate,
            out_ch,
            wake: Arc::new(wake),
        });
        let stream = device.and_then(|(dev, cfg)| match build_stream(&dev, &cfg, shared.clone()) {
            Ok(s) => Some(s),
            Err(e) => {
                eprintln!("audio: {e}");
                None
            }
        });
        Engine { shared, stream, tx: Mutex::new(None) }
    }

    /// Stop whatever plays, open `s`, seek to `start_secs`, start playing.
    pub fn load(&self, s: StreamInfo, start_secs: f32) {
        if self.stream.is_none() {
            return self.shared.emit(Event::Error("no audio output device".into()));
        }
        let epoch = {
            let mut r = self.shared.ring.lock().unwrap();
            r.duration = None;
            r.restart(start_secs as f64)
        };
        let (tx, rx) = mpsc::channel();
        if start_secs > 0.0 {
            let _ = tx.send(start_secs);
        }
        // Dropping the previous sender ends its decode thread.
        *self.tx.lock().unwrap() = Some(tx);
        self.shared.playing.store(true, Relaxed);
        let shared = self.shared.clone();
        thread::spawn(move || {
            if let Err(e) = run(&shared, &rx, &s, epoch) {
                shared.fail(epoch, e);
            }
        });
        (self.shared.wake)();
    }

    pub fn play(&self) {
        self.shared.playing.store(true, Relaxed);
        (self.shared.wake)();
    }

    pub fn pause(&self) {
        self.shared.playing.store(false, Relaxed);
        (self.shared.wake)();
    }

    pub fn seek(&self, secs: f32) {
        self.shared.ring.lock().unwrap().restart(secs as f64);
        if let Some(tx) = self.tx.lock().unwrap().as_ref() {
            let _ = tx.send(secs);
        }
        (self.shared.wake)();
    }

    /// Slider position 0..=1. Loudness is heard logarithmically, so the gain is its cube (the curve PulseAudio uses):
    /// a linear gain puts nearly all the audible change in the bottom tenth of the slider.
    pub fn set_volume(&self, v: f32) {
        self.shared.volume.store(v.clamp(0.0, 1.0).powi(3).to_bits(), Relaxed);
    }

    pub fn stop(&self) {
        *self.tx.lock().unwrap() = None;
        self.shared.playing.store(false, Relaxed);
        {
            let mut r = self.shared.ring.lock().unwrap();
            r.duration = None;
            r.restart(0.0);
        }
        (self.shared.wake)();
    }

    /// Seconds actually played out (output-callback frames since the last load/seek, plus the base).
    pub fn position(&self) -> f32 {
        let r = self.shared.ring.lock().unwrap();
        (r.base + r.played as f64 / self.shared.out_rate as f64) as f32
    }

    pub fn duration(&self) -> Option<f32> {
        self.shared.ring.lock().unwrap().duration
    }

    pub fn is_playing(&self) -> bool {
        self.shared.playing.load(Relaxed)
    }

    pub fn poll_events(&self) -> Vec<Event> {
        std::mem::take(&mut *self.shared.events.lock().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use super::chunk_range;

    #[test]
    fn chunk_range_math() {
        let mib = 1u64 << 20;
        assert_eq!(chunk_range(2_500_000, mib, None), (2 * mib, 3 * mib - 1));
        assert_eq!(chunk_range(0, mib, Some(100)), (0, 99));
        assert_eq!(chunk_range(mib - 1, mib, Some(5 * mib)), (0, mib - 1));
        assert_eq!(chunk_range(4_500_000, mib, Some(4_600_000)), (4 * mib, 4_599_999));
    }

    /// Live: daemon + network + sound device. `cargo test -p metrolist-egui live_playback -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_playback() {
        let id = std::env::var("VIDEO_ID").unwrap_or_else(|_| "dQw4w9WgXcQ".into());
        let info: super::StreamInfo = crate::backend::get(&format!("/stream/{id}")).expect("resolve");
        println!("mime {} client {}", info.mime_type, info.client);
        let e = super::Engine::new(|| {});
        e.load(info, 0.);
        std::thread::sleep(std::time::Duration::from_secs(5));
        let p = e.position();
        e.seek(90.);
        std::thread::sleep(std::time::Duration::from_secs(3));
        let events = e.poll_events();
        let errs: Vec<_> = events.iter().filter_map(|v| if let super::Event::Error(s) = v { Some(s.clone()) } else { None }).collect();
        println!("pos after 5s {p:.2}, after seek {:.2}, duration {:?}, errors {errs:?}", e.position(), e.duration());
        crate::backend::stop();
        assert!(errs.is_empty() && p > 3. && e.position() > 90.5);
    }
}
