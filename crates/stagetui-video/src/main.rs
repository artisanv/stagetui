//! The video add-on for stagetui: `StagePane("video", path="movie.mp4")`.
//!
//! Pictures and sound are decoded by ffmpeg — the copy that comes with the
//! `imageio-ffmpeg` Python package (installed with this add-on), or one on the
//! `PATH`. Sound plays through the system's audio with `cpal`. The sound is the
//! clock; pictures follow it (or a plain clock when there is no sound).
//!
//! Keys while the pane has focus: Space pause, ← → seek 5 s (Shift: 30 s),
//! ↑ ↓ volume, M mute, Home start over. Click: pause. Wheel: volume.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stagetui_plugin_api::{Describe, Frame, Mouse, PaneSource, Ready, Start, serve};

/// Largest picture decoded (the terminal scales it to the pane).
const MAX_SIZE: (u32, u32) = (1280, 720);
const MAX_FPS: f64 = 30.0;

fn main() {
    std::process::exit(serve(Video::default()));
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

// ----- finding ffmpeg --------------------------------------------------------

fn on_path(program: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{program}.exe")
    } else {
        program.to_string()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(&exe))
        .find(|p| p.is_file())
}

/// ffmpeg: `STAGETUI_FFMPEG`, then imageio-ffmpeg's copy (next to this program's
/// Python environment, or asked from Python), then one on the `PATH`.
fn find_ffmpeg() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("STAGETUI_FFMPEG").filter(|v| !v.is_empty()) {
        return Some(p.into());
    }
    let exe = std::env::current_exe().ok()?;
    let bin_dir = exe.parent()?;
    if let Some(env_dir) = bin_dir.parent() {
        let mut sites = vec![env_dir.join("Lib").join("site-packages")];
        if let Ok(entries) = std::fs::read_dir(env_dir.join("lib")) {
            sites.extend(
                entries
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("python"))
                    .map(|e| e.path().join("site-packages")),
            );
        }
        for site in sites {
            let Ok(entries) = std::fs::read_dir(site.join("imageio_ffmpeg").join("binaries"))
            else {
                continue;
            };
            if let Some(found) = entries.flatten().map(|e| e.path()).find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("ffmpeg") && !n.ends_with(".md"))
            }) {
                return Some(found);
            }
        }
    }
    // Ask the Python next to us (covers per-user installs).
    for python in ["python", "python3"] {
        let candidate = bin_dir.join(if cfg!(windows) {
            format!("{python}.exe")
        } else {
            python.to_string()
        });
        if !candidate.is_file() {
            continue;
        }
        if let Ok(out) = Command::new(&candidate)
            .args([
                "-c",
                "import imageio_ffmpeg; print(imageio_ffmpeg.get_ffmpeg_exe())",
            ])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            && out.status.success()
        {
            let path = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
            if path.is_file() {
                return Some(path);
            }
        }
    }
    on_path("ffmpeg")
}

// ----- what is in the file ---------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct Media {
    duration: Option<f64>,
    size: Option<(u32, u32)>,
    fps: f64,
    audio: bool,
}

/// `HH:MM:SS.ss` → seconds.
fn clock_seconds(text: &str) -> Option<f64> {
    let mut parts = text.trim().split(':');
    let h: f64 = parts.next()?.parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// Read what `ffmpeg -i <file>` says about it.
fn parse_probe(text: &str) -> Result<Media, String> {
    if !text.contains("Input #0") {
        let last = text
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("ffmpeg couldn't read it");
        return Err(last.trim().to_string());
    }
    let duration = text
        .split("Duration: ")
        .nth(1)
        .and_then(|rest| rest.split(',').next())
        .and_then(clock_seconds);
    let video = text.lines().find(|l| l.contains("Video:"));
    let size = video.and_then(|line| {
        line.split([' ', ','])
            .filter_map(|tok| {
                let (w, h) = tok.split_once('x')?;
                if w.starts_with('0') {
                    return None; // codec tags like 0x31637661
                }
                Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))
            })
            .find(|(w, h)| *w >= 2 && *h >= 2)
    });
    let fps = video
        .and_then(|line| {
            let pieces: Vec<&str> = line.split(", ").collect();
            ["fps", "tbr"].iter().find_map(|unit| {
                pieces.iter().find_map(|p| {
                    p.trim()
                        .strip_suffix(unit)
                        .and_then(|n| n.trim().trim_end_matches('k').parse::<f64>().ok())
                })
            })
        })
        .filter(|f| *f > 0.0 && f.is_finite())
        .unwrap_or(25.0);
    let audio = text.lines().any(|l| l.contains("Audio:"));
    if size.is_none() && !audio {
        return Err("it has no pictures or sound stagetui can play".into());
    }
    Ok(Media {
        duration,
        size,
        fps,
        audio,
    })
}

fn probe(ffmpeg: &Path, src: &str) -> Result<Media, String> {
    let out = Command::new(ffmpeg)
        .args(["-hide_banner", "-nostdin", "-i", src])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("ffmpeg didn't run: {e}"))?;
    parse_probe(&String::from_utf8_lossy(&out.stderr))
}

/// The decoded picture size: the video's shape, fitted inside the pane (and
/// never above `MAX_SIZE`); even numbers, which video filters like.
fn fit(video: (u32, u32), pane: (u32, u32)) -> (u32, u32) {
    let (vw, vh) = (f64::from(video.0.max(1)), f64::from(video.1.max(1)));
    let bw = f64::from(pane.0.clamp(2, MAX_SIZE.0));
    let bh = f64::from(pane.1.clamp(2, MAX_SIZE.1));
    let scale = (bw / vw).min(bh / vh);
    let even = |v: f64| ((v.round() as u32) / 2 * 2).max(2);
    (even(vw * scale), even(vh * scale))
}

fn mm_ss(seconds: f64) -> String {
    let s = seconds.max(0.0) as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{:02}:{:02}", s / 60, s % 60)
    }
}

// ----- pictures --------------------------------------------------------------

struct Pictures {
    child: Child,
    rx: Receiver<Vec<u8>>,
    size: (u32, u32),
    fps: f64,
    start: f64,
    /// Frames taken so far.
    count: u64,
}

impl Drop for Pictures {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Pictures {
    fn spawn(
        ffmpeg: &Path,
        src: &str,
        start: f64,
        size: (u32, u32),
        fps: f64,
    ) -> Result<Self, String> {
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        if start > 0.0 {
            cmd.args(["-ss", &format!("{start:.3}")]);
        }
        cmd.args(["-i", src, "-an", "-sn", "-dn", "-vf"])
            .arg(format!(
                "scale={}:{}:flags=bilinear,fps={fps}",
                size.0, size.1
            ))
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("ffmpeg didn't start: {e}"))?;
        let mut out = child.stdout.take().ok_or("ffmpeg has no output")?;
        let bytes = size.0 as usize * size.1 as usize * 4;
        // A few frames ahead, no more: ffmpeg waits when we do.
        let (tx, rx) = mpsc::sync_channel(4);
        std::thread::spawn(move || {
            loop {
                let mut frame = vec![0u8; bytes];
                if out.read_exact(&mut frame).is_err() || tx.send(frame).is_err() {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            rx,
            size,
            fps,
            start,
            count: 0,
        })
    }

    /// When the next frame should show (media time).
    fn next_at(&self) -> f64 {
        self.start + self.count as f64 / self.fps
    }
}

// ----- sound -----------------------------------------------------------------

/// Shared between the decoder thread, the audio callback and the plugin.
struct Shared {
    ring: Mutex<VecDeque<f32>>,
    /// Sample frames played since the last seek.
    played: AtomicU64,
    paused: AtomicBool,
    /// `f32` bits; 0 when muted.
    volume: AtomicU32,
    /// Bumped on every seek: an old decoder stops feeding.
    generation: AtomicU64,
    /// The decoder of this generation reached the end.
    done: AtomicBool,
    rate: u32,
    channels: u16,
}

struct Sound {
    _stream: cpal::Stream,
    shared: Arc<Shared>,
    decoder: Option<Child>,
}

impl Drop for Sound {
    fn drop(&mut self) {
        if let Some(mut c) = self.decoder.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn write_samples<T: cpal::SizedSample + cpal::FromSample<f32>>(data: &mut [T], shared: &Shared) {
    let gain = f32::from_bits(shared.volume.load(Ordering::Relaxed));
    if shared.paused.load(Ordering::Relaxed) {
        data.fill(T::from_sample(0.0));
        return;
    }
    let mut ring = lock(&shared.ring);
    let mut taken = 0u64;
    for s in data.iter_mut() {
        match ring.pop_front() {
            Some(v) => {
                *s = T::from_sample(v * gain);
                taken += 1;
            }
            None => *s = T::from_sample(0.0),
        }
    }
    drop(ring);
    shared
        .played
        .fetch_add(taken / u64::from(shared.channels.max(1)), Ordering::Relaxed);
}

impl Sound {
    /// Open the system's default speakers (`None` if there are none).
    fn open(volume: f32) -> Result<Self, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("no speakers found")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let config: cpal::StreamConfig = supported.into();
        let shared = Arc::new(Shared {
            ring: Mutex::new(VecDeque::new()),
            played: AtomicU64::new(0),
            paused: AtomicBool::new(false),
            volume: AtomicU32::new(volume.to_bits()),
            generation: AtomicU64::new(0),
            done: AtomicBool::new(false),
            rate: config.sample_rate,
            channels: config.channels,
        });
        let err = |_e: cpal::Error| {};
        macro_rules! build {
            ($t:ty) => {{
                let s = Arc::clone(&shared);
                device.build_output_stream::<$t, _, _>(
                    config.clone(),
                    move |data: &mut [$t], _: &cpal::OutputCallbackInfo| write_samples(data, &s),
                    err,
                    None,
                )
            }};
        }
        let stream = match format {
            cpal::SampleFormat::F32 => build!(f32),
            cpal::SampleFormat::I16 => build!(i16),
            cpal::SampleFormat::I32 => build!(i32),
            cpal::SampleFormat::U16 => build!(u16),
            cpal::SampleFormat::F64 => build!(f64),
            other => return Err(format!("speaker format {other:?} isn't supported")),
        }
        .map_err(|e| e.to_string())?;
        stream.play().map_err(|e| e.to_string())?;
        Ok(Self {
            _stream: stream,
            shared,
            decoder: None,
        })
    }

    /// Start decoding the sound at `start` seconds (after a seek).
    fn feed(&mut self, ffmpeg: &Path, src: &str, start: f64) -> Result<(), String> {
        if let Some(mut c) = self.decoder.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
        let epoch = self.shared.generation.fetch_add(1, Ordering::SeqCst) + 1;
        lock(&self.shared.ring).clear();
        self.shared.played.store(0, Ordering::SeqCst);
        self.shared.done.store(false, Ordering::SeqCst);
        let mut cmd = Command::new(ffmpeg);
        cmd.args(["-hide_banner", "-loglevel", "error", "-nostdin"]);
        if start > 0.0 {
            cmd.args(["-ss", &format!("{start:.3}")]);
        }
        cmd.args(["-i", src, "-vn", "-sn", "-dn", "-f", "f32le", "-ac"])
            .arg(self.shared.channels.to_string())
            .arg("-ar")
            .arg(self.shared.rate.to_string())
            .arg("pipe:1")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("ffmpeg didn't start: {e}"))?;
        let mut out = child.stdout.take().ok_or("ffmpeg has no output")?;
        let shared = Arc::clone(&self.shared);
        // Keep about two seconds ready.
        let ahead = shared.rate as usize * usize::from(shared.channels) * 2;
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            let mut carry: Vec<u8> = Vec::new();
            loop {
                if shared.generation.load(Ordering::SeqCst) != epoch {
                    return;
                }
                if lock(&shared.ring).len() > ahead {
                    std::thread::sleep(Duration::from_millis(10));
                    continue;
                }
                let n = match out.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                carry.extend_from_slice(&buf[..n]);
                let whole = carry.len() / 4 * 4;
                let mut ring = lock(&shared.ring);
                if shared.generation.load(Ordering::SeqCst) != epoch {
                    return;
                }
                let (samples, _) = carry[..whole].as_chunks::<4>();
                ring.extend(samples.iter().map(|b| f32::from_le_bytes(*b)));
                drop(ring);
                carry.drain(..whole);
            }
            if shared.generation.load(Ordering::SeqCst) == epoch {
                shared.done.store(true, Ordering::SeqCst);
            }
        });
        self.decoder = Some(child);
        Ok(())
    }

    /// Media seconds played since the last seek.
    fn played(&self) -> f64 {
        self.shared.played.load(Ordering::SeqCst) as f64 / f64::from(self.shared.rate.max(1))
    }

    /// The sound ran out (shorter than the pictures, or the end).
    fn finished(&self) -> bool {
        self.shared.done.load(Ordering::SeqCst) && lock(&self.shared.ring).is_empty()
    }
}

// ----- the add-on ------------------------------------------------------------

struct Video {
    ffmpeg: Option<PathBuf>,
    src: String,
    media: Option<Media>,
    pane: (u32, u32),
    pictures: Option<Pictures>,
    sound: Option<Sound>,
    /// No speakers (or `STAGETUI_VIDEO_NO_AUDIO`): a plain clock instead.
    quiet: Option<String>,
    /// Where the last seek went (media seconds).
    start: f64,
    /// Plain clock: media time at `anchor`.
    base: f64,
    anchor: Instant,
    paused: bool,
    looped: bool,
    volume: f32,
    muted: bool,
    ended: bool,
    /// Show the next decoded frame even if it isn't due (after a seek or resize).
    fresh: bool,
    /// The sound ran out early: follow the plain clock from here.
    sound_gone: bool,
    resize_at: Option<Instant>,
    status: String,
    status_due: Instant,
    events: Vec<(String, Value)>,
    blank_sent: bool,
}

impl Default for Video {
    fn default() -> Self {
        Self {
            ffmpeg: None,
            src: String::new(),
            media: None,
            pane: (640, 360),
            pictures: None,
            sound: None,
            quiet: None,
            start: 0.0,
            base: 0.0,
            anchor: Instant::now(),
            paused: false,
            looped: false,
            volume: 1.0,
            muted: false,
            ended: false,
            fresh: true,
            sound_gone: false,
            resize_at: None,
            status: String::new(),
            status_due: Instant::now(),
            events: Vec::new(),
            blank_sent: false,
        }
    }
}

impl Video {
    fn gain(&self) -> f32 {
        if self.muted { 0.0 } else { self.volume }
    }

    fn duration(&self) -> Option<f64> {
        self.media.as_ref().and_then(|m| m.duration)
    }

    /// Where we are, in media seconds.
    fn position(&self) -> f64 {
        let pos = match &self.sound {
            Some(s) if !self.sound_gone => self.start + s.played(),
            _ if self.paused => self.base,
            _ => self.base + self.anchor.elapsed().as_secs_f64(),
        };
        self.duration().map_or(pos, |d| pos.min(d))
    }

    fn picture_fps(&self) -> f64 {
        self.media.as_ref().map_or(25.0, |m| m.fps.min(MAX_FPS))
    }

    fn start_pictures(&mut self, at: f64) {
        self.pictures = None;
        let (Some(ffmpeg), Some(size)) = (&self.ffmpeg, self.media.as_ref().and_then(|m| m.size))
        else {
            return;
        };
        let out = fit(size, self.pane);
        self.pictures = Pictures::spawn(ffmpeg, &self.src, at, out, self.picture_fps()).ok();
        self.fresh = true;
    }

    /// Go to `at` seconds: both decoders start over there.
    fn seek(&mut self, at: f64) {
        let at = self
            .duration()
            .map_or(at, |d| at.min((d - 0.1).max(0.0)))
            .max(0.0);
        self.start = at;
        self.base = at;
        self.anchor = Instant::now();
        self.ended = false;
        self.sound_gone = false;
        self.start_pictures(at);
        if let (Some(sound), Some(ffmpeg)) = (&mut self.sound, &self.ffmpeg)
            && self.media.as_ref().is_some_and(|m| m.audio)
            && sound.feed(ffmpeg, &self.src, at).is_err()
        {
            self.sound_gone = true;
        }
        self.status_due = Instant::now();
    }

    fn set_paused(&mut self, paused: bool) {
        if paused == self.paused {
            return;
        }
        if paused {
            self.base = self.position();
        } else {
            self.anchor = Instant::now();
            if self.ended {
                self.seek(0.0);
            }
        }
        self.paused = paused;
        if let Some(s) = &self.sound {
            s.shared.paused.store(paused, Ordering::SeqCst);
        }
        self.events.push((
            "state".into(),
            json!(if paused { "paused" } else { "playing" }),
        ));
        self.status_due = Instant::now();
    }

    fn set_volume(&mut self, volume: f32) {
        self.volume = volume.clamp(0.0, 1.0);
        if let Some(s) = &self.sound {
            s.shared
                .volume
                .store(self.gain().to_bits(), Ordering::SeqCst);
        }
        self.status_due = Instant::now();
    }

    fn set_muted(&mut self, muted: bool) {
        self.muted = muted;
        self.set_volume(self.volume);
    }

    fn state(&self) -> Value {
        json!({
            "path": self.src,
            "position": (self.position() * 100.0).round() / 100.0,
            "duration": self.duration(),
            "paused": self.paused,
            "ended": self.ended,
            "volume": self.volume,
            "muted": self.muted,
            "sound": self.sound.is_some() && !self.sound_gone,
        })
    }

    fn status_line(&self) -> String {
        let pos = mm_ss(self.position());
        let total = self.duration().map_or_else(|| "live".to_string(), mm_ss);
        let state = if self.ended {
            "■"
        } else if self.paused {
            "‖"
        } else {
            "▶"
        };
        let sound = match (&self.quiet, self.muted) {
            (Some(why), _) => format!("no sound ({why})"),
            (None, true) => "muted".to_string(),
            (None, false) => format!("volume {}%", (self.volume * 100.0).round()),
        };
        format!("{state} {pos} / {total} · {sound} · space ‖ · ← → seek · ↑ ↓ volume · m mute")
    }
}

impl PaneSource for Video {
    fn describe(&self) -> Describe {
        let mut d = Describe::new("video", "Video", "play");
        d.version = env!("CARGO_PKG_VERSION").into();
        d
    }

    fn start(&mut self, start: &Start) -> Result<Ready, String> {
        let args = &start.args;
        self.src = ["path", "url", "src"]
            .iter()
            .find_map(|k| args.get(*k).and_then(Value::as_str))
            .unwrap_or_default()
            .to_string();
        if self.src.is_empty() {
            return Err("no video given: StagePane(\"video\", path=\"movie.mp4\")".into());
        }
        let is_url = self.src.contains("://");
        if !is_url && !Path::new(&self.src).exists() {
            return Err(format!("\"{}\" was not found", self.src));
        }
        let ffmpeg = find_ffmpeg()
            .ok_or("ffmpeg wasn't found — it comes with the add-on (pip install imageio-ffmpeg)")?;
        let media = probe(&ffmpeg, &self.src)?;
        self.ffmpeg = Some(ffmpeg);
        self.pane = (start.width.max(2), start.height.max(2));
        self.looped = args.get("loop").and_then(Value::as_bool).unwrap_or(false);
        self.muted = args.get("muted").and_then(Value::as_bool).unwrap_or(false);
        self.volume = args
            .get("volume")
            .and_then(Value::as_f64)
            .map_or(1.0, |v| v as f32)
            .clamp(0.0, 1.0);
        let fps = media.fps.min(MAX_FPS).ceil() as u32;
        let has_audio = media.audio;
        self.media = Some(media);
        if has_audio {
            if std::env::var("STAGETUI_VIDEO_NO_AUDIO").is_ok_and(|v| v == "1") {
                self.quiet = Some("turned off".into());
            } else {
                match Sound::open(self.gain()) {
                    Ok(s) => self.sound = Some(s),
                    Err(why) => self.quiet = Some(why),
                }
            }
        }
        let autoplay = args
            .get("autoplay")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let at = args.get("start").and_then(Value::as_f64).unwrap_or(0.0);
        self.seek(at);
        if !autoplay {
            self.set_paused(true);
        }
        self.events.clear();
        Ok(Ready {
            keys: [
                "space",
                "left",
                "right",
                "shift+left",
                "shift+right",
                "up",
                "down",
                "m",
                "home",
            ]
            .map(String::from)
            .to_vec(),
            text: false,
            fps: fps.clamp(5, 30),
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.pane = (width.max(2), height.max(2));
        // Wait for the window to settle before restarting the decoder.
        self.resize_at = Some(Instant::now() + Duration::from_millis(250));
    }

    fn frame(&mut self, _now: Duration) -> Option<Frame> {
        if self.resize_at.is_some_and(|t| Instant::now() >= t) {
            self.resize_at = None;
            let want = self
                .media
                .as_ref()
                .and_then(|m| m.size)
                .map(|s| fit(s, self.pane));
            if want.is_some() && want != self.pictures.as_ref().map(|p| p.size) {
                let at = self.position();
                self.start_pictures(at);
            }
        }
        if let Some(s) = &self.sound
            && !self.sound_gone
            && !self.paused
            && s.finished()
        {
            // The sound ended first: keep the pictures going on a plain clock.
            self.base = self.position();
            self.anchor = Instant::now();
            self.sound_gone = true;
        }
        let pos = self.position();
        let mut shown = None;
        let mut pictures_done = self.pictures.is_none();
        if let Some(p) = &mut self.pictures {
            while self.fresh || (!self.paused && p.next_at() <= pos + 0.001) {
                match p.rx.try_recv() {
                    Ok(f) => {
                        p.count += 1;
                        shown = Some(Frame {
                            width: p.size.0,
                            height: p.size.1,
                            rgba: f,
                        });
                        if self.fresh {
                            self.fresh = false;
                            break;
                        }
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        pictures_done = true;
                        break;
                    }
                }
            }
        }
        if self.media.as_ref().is_some_and(|m| m.size.is_none()) && !self.blank_sent {
            // Sound only: a quiet dark picture instead of "Starting…".
            self.blank_sent = true;
            shown = Some(Frame::from_fn(16, 9, |_, _| [16, 16, 30, 255]));
        }
        let sound_done = self
            .sound
            .as_ref()
            .is_none_or(|s| self.sound_gone || s.finished());
        let at_end = self.duration().is_some_and(|d| pos >= d - 0.05);
        // The known end, or (live streams) both decoders ran dry.
        if !self.ended && !self.paused && (at_end || (pictures_done && sound_done)) {
            if self.looped {
                self.seek(0.0);
            } else {
                self.base = pos;
                self.ended = true;
                self.paused = true;
                if let Some(s) = &self.sound {
                    s.shared.paused.store(true, Ordering::SeqCst);
                }
                self.events.push(("ended".into(), json!(self.src)));
                self.status_due = Instant::now();
            }
        }
        shown
    }

    fn key(&mut self, key: &str, _text: Option<&str>) -> bool {
        let pos = self.position();
        match key {
            "space" => self.set_paused(!self.paused),
            "left" => self.seek(pos - 5.0),
            "right" => self.seek(pos + 5.0),
            "shift+left" => self.seek(pos - 30.0),
            "shift+right" => self.seek(pos + 30.0),
            "up" => self.set_volume(self.volume + 0.1),
            "down" => self.set_volume(self.volume - 0.1),
            "m" => self.set_muted(!self.muted),
            "home" => self.seek(0.0),
            _ => return false,
        }
        true
    }

    fn mouse(&mut self, m: &Mouse) {
        match m.kind.as_str() {
            "down" if m.button.as_deref() == Some("left") => self.set_paused(!self.paused),
            "scroll_up" => self.set_volume(self.volume + 0.05),
            "scroll_down" => self.set_volume(self.volume - 0.05),
            _ => {}
        }
    }

    fn call(&mut self, method: &str, args: &Value) -> Result<Value, String> {
        let number = |v: &Value| {
            v.as_f64()
                .or_else(|| v.get("value").and_then(Value::as_f64))
        };
        match method {
            "play" => self.set_paused(false),
            "pause" => self.set_paused(true),
            "toggle" => self.set_paused(!self.paused),
            "seek" => self.seek(
                number(args)
                    .or_else(|| args.get("seconds").and_then(Value::as_f64))
                    .ok_or("seek needs seconds, e.g. call(\"seek\", 42)")?,
            ),
            "volume" => self.set_volume(number(args).ok_or("volume needs 0..1")? as f32),
            "mute" => self.set_muted(args.as_bool().unwrap_or(!self.muted)),
            "state" => {}
            other => return Err(format!("the video add-on has no method \"{other}\"")),
        }
        Ok(self.state())
    }

    fn status(&mut self) -> Option<String> {
        if Instant::now() < self.status_due {
            return None;
        }
        self.status_due = Instant::now() + Duration::from_millis(500);
        let line = self.status_line();
        (line != self.status).then(|| {
            self.status = line.clone();
            line
        })
    }

    fn events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.events)
    }

    fn stop(&mut self) {
        self.pictures = None;
        self.sound = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &str = "Input #0, matroska,webm, from 'مَرْحَبًا Hello 日本語 नमस्ते.mkv':
  Duration: 00:01:23.45, start: 0.000000, bitrate: 512 kb/s
  Stream #0:0: Video: h264 (High) (avc1 / 0x31637661), yuv420p(progressive), 1920x1080 [SAR 1:1 DAR 16:9], 29.97 fps, 29.97 tbr, 1k tbn
  Stream #0:1(eng): Audio: aac (LC), 48000 Hz, stereo, fltp
At least one output file must be specified";

    #[test]
    fn reads_what_ffmpeg_says() {
        let m = parse_probe(PROBE).unwrap();
        assert_eq!(m.size, Some((1920, 1080)));
        assert!((m.duration.unwrap() - 83.45).abs() < 1e-9);
        assert!((m.fps - 29.97).abs() < 1e-9);
        assert!(m.audio);
        let live = parse_probe(
            "Input #0, mpegts\n  Duration: N/A, start: 1\n  Stream #0:0: Audio: mp3, 44100 Hz",
        )
        .unwrap();
        assert_eq!((live.duration, live.size, live.audio), (None, None, true));
        let missing = parse_probe("nope.mp4: No such file or directory").unwrap_err();
        assert!(missing.contains("No such file"), "{missing}");
    }

    #[test]
    fn sizes_and_times() {
        assert_eq!(fit((1920, 1080), (800, 800)), (800, 450));
        assert_eq!(
            fit((1920, 1080), (4000, 4000)),
            (1280, 720),
            "never above the cap"
        );
        assert_eq!(fit((101, 51), (101, 51)), (100, 50), "even sizes");
        assert_eq!(mm_ss(83.45), "01:23");
        assert_eq!(mm_ss(3725.0), "1:02:05");
        assert_eq!(clock_seconds("00:01:23.45"), Some(83.45));
    }
}
