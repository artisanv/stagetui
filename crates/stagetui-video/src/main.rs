//! The video add-on for stagetui: `StagePane("video", path="movie.mp4")`.
//!
//! Pictures and sound are decoded by ffmpeg — the copy that comes with the
//! `imageio-ffmpeg` Python package (installed with this add-on), or one on the
//! `PATH`. Sound plays through the system's audio with `cpal`. The sound is the
//! clock; pictures follow it (or a plain clock when there is no sound).
//!
//! Keys while the pane has focus: Space pause, ← → seek 5 s (Shift: 30 s),
//! ↑ ↓ volume, M mute, Home start over, Enter or O type another file's path
//! (the app can change them: `ACTIONS`). Click: pause. Wheel: volume.

use std::collections::VecDeque;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stagetui_plugin_api::{
    Describe, Frame, KeyAction, Keymap, Mouse, PaneSource, PathLine, Ready, Start, Theme, Typed,
    serve,
};

/// Largest picture decoded (the terminal scales it to the pane).
const MAX_SIZE: (u32, u32) = (1280, 720);
const MAX_FPS: f64 = 30.0;
const NO_FFMPEG: &str =
    "ffmpeg wasn't found — it comes with the add-on (pip install imageio-ffmpeg)";

/// What the keys do; the app can give them other keys:
/// `StagePane("video", keys={"play_pause": "k", "mute": None})`.
const ACTIONS: &[KeyAction] = &[
    KeyAction {
        name: "open",
        keys: &["enter", "o"],
        about: "type a video's file path or link, then Enter plays it",
    },
    KeyAction {
        name: "play_pause",
        keys: &["space"],
        about: "pause, or play on",
    },
    KeyAction {
        name: "back",
        keys: &["left"],
        about: "back 5 seconds",
    },
    KeyAction {
        name: "forward",
        keys: &["right"],
        about: "forward 5 seconds",
    },
    KeyAction {
        name: "back_more",
        keys: &["shift+left"],
        about: "back 30 seconds",
    },
    KeyAction {
        name: "forward_more",
        keys: &["shift+right"],
        about: "forward 30 seconds",
    },
    KeyAction {
        name: "louder",
        keys: &["up"],
        about: "volume up",
    },
    KeyAction {
        name: "quieter",
        keys: &["down"],
        about: "volume down",
    },
    KeyAction {
        name: "mute",
        keys: &["m"],
        about: "sound off, or on again",
    },
    KeyAction {
        name: "restart",
        keys: &["home"],
        about: "start over from the beginning",
    },
];

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
    /// Out of sight (another pane type or tab is showing): `Some(play when back)`.
    /// Nothing plays, and no sound is heard, while it's hidden.
    hidden: Option<bool>,
    /// Which key does what (the app can change them).
    keys: Keymap,
    /// The line for typing a video's path (open: the plugin wants every key).
    line: PathLine,
    /// No video: why the last one didn't open, and what it was (to type it again).
    problem: Option<(String, String)>,
    autoplay: bool,
    theme: Theme,
    /// What stagetui was last told the plugin wants.
    sent: Option<Ready>,
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
            hidden: None,
            keys: Keymap::new(ACTIONS, &Value::Null).unwrap_or_default(),
            line: PathLine::default(),
            problem: None,
            autoplay: true,
            theme: Theme::default(),
            sent: None,
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
        if self.ended {
            return self.base;
        }
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

    /// Playing, or will be once it's back in sight.
    fn playing(&self) -> bool {
        self.hidden.unwrap_or(!self.paused)
    }

    fn set_paused(&mut self, paused: bool) {
        if let Some(resume) = &mut self.hidden {
            // Out of sight: stay quiet; play (or not) once it's back.
            *resume = !paused;
            return;
        }
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

    /// Open a file or link and play it from `at` seconds (unless `autoplay=False`).
    fn open(&mut self, src: &str, at: f64) -> Result<(), String> {
        if !src.contains("://") && !Path::new(src).exists() {
            return Err(format!("\"{src}\" was not found"));
        }
        let ffmpeg = self.ffmpeg.clone().ok_or(NO_FFMPEG)?;
        let media = probe(&ffmpeg, src)?;
        // What was playing stops (its sound too).
        self.pictures = None;
        self.sound = None;
        self.quiet = None;
        self.src = src.to_string();
        self.problem = None;
        self.blank_sent = false;
        let has_audio = media.audio;
        self.media = Some(media);
        if has_audio {
            if std::env::var("STAGETUI_VIDEO_NO_AUDIO").is_ok_and(|v| v == "1") {
                self.quiet = Some("turned off".into());
            } else {
                match Sound::open(self.gain()) {
                    Ok(s) => {
                        // Already paused if the pane was hidden before it started.
                        s.shared.paused.store(self.paused, Ordering::SeqCst);
                        self.sound = Some(s);
                    }
                    Err(why) => self.quiet = Some(why),
                }
            }
        }
        self.seek(at);
        self.set_paused(!self.autoplay);
        Ok(())
    }

    /// What stagetui should send: every key while the path line is open.
    fn wanted(&self) -> Ready {
        let fps = self
            .media
            .as_ref()
            .map_or(30, |m| m.fps.min(MAX_FPS).ceil() as u32)
            .clamp(5, 30);
        if self.line.is_open() {
            return self.line.ready(fps);
        }
        Ready {
            keys: self.keys.all_keys(),
            text: false,
            fps,
        }
    }

    /// Open the path line: what failed, or the playing file's folder.
    fn ask_for_a_path(&mut self) {
        let folder = Path::new(&self.src)
            .parent()
            .filter(|p| !self.src.contains("://") && !p.as_os_str().is_empty())
            .map_or_else(String::new, |p| {
                format!("{}{}", p.display(), std::path::MAIN_SEPARATOR)
            });
        let typed = self
            .problem
            .as_ref()
            .map_or(folder, |(_, tried)| tried.clone());
        self.line.open(&typed);
        self.status_due = Instant::now();
    }

    fn status_line(&self) -> String {
        if self.line.is_open() {
            return self.line.status("Open a video");
        }
        if self.media.is_none() {
            let ask = self.keys.spoken("open");
            return match (&self.problem, ask) {
                (Some((why, _)), Some(key)) => {
                    format!("⚠ {why} — press {key} to type another path")
                }
                (Some((why, _)), None) => format!("⚠ {why}"),
                (None, Some(key)) => {
                    format!("No video yet — press {key} to type a file path")
                }
                (None, None) => "No video yet".into(),
            };
        }
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
        let mut line = format!("{state} {pos} / {total} · {sound}");
        // The keys the app gave (a key turned off isn't shown); space offers what it does.
        let toggle = if self.playing() { "‖" } else { "▶" };
        for (actions, what) in [
            (&["play_pause"][..], toggle),
            (&["back", "forward"][..], "seek"),
            (&["louder", "quieter"][..], "volume"),
            (&["mute"][..], "mute"),
        ] {
            let keys: Vec<String> = actions.iter().filter_map(|a| self.keys.label(a)).collect();
            if !keys.is_empty() {
                line.push_str(&format!(" · {} {what}", keys.join(" ")));
            }
        }
        line
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
        self.keys = Keymap::new(ACTIONS, args)?;
        self.ffmpeg = Some(find_ffmpeg().ok_or(NO_FFMPEG)?);
        self.theme = start.theme.clone();
        self.pane = (start.width.max(2), start.height.max(2));
        self.looped = args.get("loop").and_then(Value::as_bool).unwrap_or(false);
        self.muted = args.get("muted").and_then(Value::as_bool).unwrap_or(false);
        self.volume = args
            .get("volume")
            .and_then(Value::as_f64)
            .map_or(1.0, |v| v as f32)
            .clamp(0.0, 1.0);
        self.autoplay = args
            .get("autoplay")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        // No video, or one that won't open: the pane says so and asks for a path.
        if let Some(src) = ["path", "url", "src"]
            .iter()
            .find_map(|k| args.get(*k).and_then(Value::as_str))
            .filter(|s| !s.is_empty())
        {
            let at = args.get("start").and_then(Value::as_f64).unwrap_or(0.0);
            if let Err(why) = self.open(src, at) {
                self.problem = Some((why, src.to_string()));
            }
        }
        self.events.clear();
        self.sent = Some(self.wanted());
        Ok(self.wanted())
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.pane = (width.max(2), height.max(2));
        if self.media.is_none() {
            self.blank_sent = false; // the quiet picture again, at the new size
        }
        // Wait for the window to settle before restarting the decoder.
        self.resize_at = Some(Instant::now() + Duration::from_millis(250));
    }

    fn frame(&mut self, _now: Duration) -> Option<Frame> {
        if self.media.is_none() {
            // Nothing to play yet: a quiet picture (the status line says what to do).
            return (!std::mem::replace(&mut self.blank_sent, true))
                .then(|| Frame::placeholder(self.pane.0, self.pane.1, &self.theme));
        }
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
                // The end: the clock shows all of it (04:56 / 04:56).
                self.base = self.duration().unwrap_or(pos);
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

    fn key(&mut self, key: &str, text: Option<&str>) -> bool {
        if self.line.is_open() {
            if let Typed::Open(src) = self.line.key(key, text) {
                match self.open(&src, 0.0) {
                    Ok(()) => self.line.close(),
                    Err(why) => self.line.fail(why),
                }
            }
            self.status_due = Instant::now();
            return true;
        }
        let Some(action) = self.keys.action(key) else {
            return false;
        };
        if self.media.is_none() && action != "open" {
            return false; // nothing to play yet
        }
        let pos = self.position();
        match action {
            "open" => self.ask_for_a_path(),
            "play_pause" => self.set_paused(self.playing()),
            "back" => self.seek(pos - 5.0),
            "forward" => self.seek(pos + 5.0),
            "back_more" => self.seek(pos - 30.0),
            "forward_more" => self.seek(pos + 30.0),
            "louder" => self.set_volume(self.volume + 0.1),
            "quieter" => self.set_volume(self.volume - 0.1),
            "mute" => self.set_muted(!self.muted),
            "restart" => self.seek(0.0),
            _ => return false,
        }
        true
    }

    fn mouse(&mut self, m: &Mouse) {
        if self.media.is_none() {
            return;
        }
        match m.kind.as_str() {
            "down" if m.button.as_deref() == Some("left") => self.set_paused(self.playing()),
            "scroll_up" => self.set_volume(self.volume + 0.05),
            "scroll_down" => self.set_volume(self.volume - 0.05),
            _ => {}
        }
    }

    /// Out of sight, a playing video pauses (sound too); back in sight, it plays
    /// on. One the user paused stays paused.
    fn shown(&mut self, shown: bool) {
        match (shown, self.hidden) {
            (false, None) => {
                let playing = !self.paused;
                self.set_paused(true);
                self.hidden = Some(playing);
            }
            (true, Some(resume)) => {
                self.hidden = None;
                if resume {
                    self.set_paused(false);
                }
            }
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
            "toggle" => self.set_paused(self.playing()),
            "seek" => self.seek(
                number(args)
                    .or_else(|| args.get("seconds").and_then(Value::as_f64))
                    .ok_or("seek needs seconds, e.g. call(\"seek\", 42)")?,
            ),
            "volume" => self.set_volume(number(args).ok_or("volume needs 0..1")? as f32),
            "mute" => self.set_muted(args.as_bool().unwrap_or(!self.muted)),
            "open" | "load" => {
                let src = args
                    .as_str()
                    .or_else(|| args.get("path").and_then(Value::as_str))
                    .ok_or("open needs a file or link, e.g. call(\"open\", \"movie.mp4\")")?;
                self.open(src, 0.0)?;
            }
            "state" => {}
            other => return Err(format!("the video add-on has no method \"{other}\"")),
        }
        Ok(self.state())
    }

    fn paste(&mut self, text: &str) {
        self.line.paste(text);
        self.status_due = Instant::now();
    }

    fn ready(&mut self) -> Option<Ready> {
        let want = self.wanted();
        (self.sent.as_ref() != Some(&want)).then(|| {
            self.sent = Some(want.clone());
            want
        })
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

    fn states(v: &mut Video) -> Vec<Value> {
        v.events()
            .into_iter()
            .filter(|(name, _)| name == "state")
            .map(|(_, value)| value)
            .collect()
    }

    /// A video that is open (sound only, no decoders: nothing really plays).
    fn opened(seconds: f64) -> Video {
        Video {
            media: Some(Media {
                duration: Some(seconds),
                size: None,
                fps: 25.0,
                audio: false,
            }),
            src: "مَرْحَبًا Hello 日本語 नमस्ते.mp4".into(),
            ..Video::default()
        }
    }

    #[test]
    fn no_video_asks_for_one() {
        // Opened from a pane's menu: no file yet. The pane says what to do, shows a
        // quiet picture, and Enter opens a line to type the path.
        let mut v = Video::default();
        assert_eq!(
            v.status_line(),
            "No video yet — press Enter to type a file path"
        );
        assert!(v.frame(Duration::ZERO).is_some() && v.frame(Duration::ZERO).is_none());
        assert!(!v.key("space", None), "nothing to play yet");
        v.mouse(&Mouse {
            kind: "down".into(),
            button: Some("left".into()),
            x: 1,
            y: 1,
        });
        assert!(v.events().is_empty(), "a click does nothing yet");
        v.sent = Some(v.wanted());
        assert!(v.key("enter", None));
        assert!(v.ready().expect("every key while typing").text);
        for c in "nope.mp4".chars() {
            v.key(&c.to_string(), Some(&c.to_string()));
        }
        v.key("enter", None);
        let status = v.status_line();
        assert!(
            status.starts_with("Open a video: nope.mp4█ · ⚠ \"nope.mp4\" was not found"),
            "{status}"
        );
        assert!(v.line.is_open(), "the line stays to fix it");
        v.key("esc", None);
        assert!(!v.ready().unwrap().text, "the usual keys again");
        // Something the app gave that didn't open: said, and offered again.
        v.problem = Some(("\"a.mp4\" was not found".into(), "a.mp4".into()));
        assert!(
            v.status_line()
                .ends_with("press Enter to type another path")
        );
        v.key("o", None);
        assert_eq!(v.line.text(), "a.mp4");
        let e = v.call("open", &json!("/nope/b.mp4")).unwrap_err();
        assert!(e.contains("was not found"), "{e}");
    }

    #[test]
    fn the_end_shows_all_of_it_and_space_offers_play() {
        let mut v = opened(2.0);
        assert!(v.status_line().contains("space ‖"), "playing: space pauses");
        v.set_paused(true);
        assert!(v.status_line().contains("space ▶"), "paused: space plays");
        v.set_paused(false);
        // Just before the end (as the clock reads it), then a frame finds the end.
        v.base = 1.97;
        v.anchor = Instant::now();
        let _ = v.frame(Duration::ZERO);
        assert!(v.ended);
        let line = v.status_line();
        assert!(line.starts_with("■ 00:02 / 00:02"), "{line}");
        assert!(line.contains("space ▶"), "{line}");
        assert_eq!(v.state()["position"], json!(2.0));
        // Space plays it again from the start.
        v.key("space", None);
        assert!(!v.ended && v.status_line().starts_with("▶ 00:00 / 00:02"));
    }

    #[test]
    fn keys_can_be_renamed_or_turned_off() {
        let mut v = opened(10.0);
        assert!(
            v.status_line()
                .ends_with("space ‖ · ← → seek · ↑ ↓ volume · m mute"),
            "as it comes: {}",
            v.status_line()
        );
        let keys = json!({"keys": {"mute": "x", "play_pause": null, "back": ["j", "left"]}});
        v.keys = Keymap::new(ACTIONS, &keys).unwrap();
        // The new key works, the old one does nothing.
        assert!(!v.key("m", None) && !v.muted);
        assert!(v.key("x", None) && v.muted);
        // Turned off: no key does it.
        assert!(!v.key("space", None) && !v.paused);
        assert!(v.keys.keys("play_pause").is_empty());
        assert!(v.key("j", None) && v.key("left", None), "both keys work");
        assert!(
            v.key("down", None) && (v.volume - 0.9).abs() < 1e-6,
            "the rest as before"
        );
        let all = v.keys.all_keys();
        assert!(all.contains(&"x".to_string()) && all.contains(&"j".to_string()));
        assert!(!all.contains(&"m".to_string()) && !all.contains(&"space".to_string()));
        // The status line shows the keys as the app named them.
        let line = v.status_line();
        assert!(line.ends_with("· j → seek · ↑ ↓ volume · x mute"), "{line}");
        assert!(!line.contains('‖'), "off: not shown: {line}");
        // Mistakes are said plainly (the pane shows it).
        let e = Keymap::new(ACTIONS, &json!({"keys": {"pause": "p"}})).unwrap_err();
        assert!(e.contains("\"pause\"") && e.contains("play_pause"), "{e}");
        let e = Keymap::new(ACTIONS, &json!({"keys": {"mute": "up"}})).unwrap_err();
        assert!(e.contains("\"louder\" and \"mute\""), "{e}");
    }

    #[test]
    fn hidden_pauses_and_comes_back() {
        // Playing when hidden: pauses, and plays on when it's back.
        let mut v = Video::default();
        v.shown(false);
        assert!(v.paused);
        assert_eq!(states(&mut v), [json!("paused")]);
        v.shown(false);
        assert!(states(&mut v).is_empty(), "only changes count");
        v.shown(true);
        assert!(!v.paused);
        assert_eq!(states(&mut v), [json!("playing")]);

        // Paused by the user: stays paused.
        v.set_paused(true);
        v.shown(false);
        v.shown(true);
        assert!(v.paused);
        assert_eq!(states(&mut v), [json!("paused")]);

        // The app asks while it's hidden: quiet now, done when it's back.
        v.shown(false);
        assert_eq!(v.call("play", &Value::Null).unwrap()["paused"], json!(true));
        v.shown(true);
        assert!(!v.paused, "play waited for the pane");
        v.shown(false);
        v.call("toggle", &Value::Null).unwrap();
        assert!(v.paused && v.hidden == Some(false));
        v.call("toggle", &Value::Null).unwrap();
        assert!(v.paused && v.hidden == Some(true), "toggle flips the wish");
        v.call("pause", &Value::Null).unwrap();
        v.shown(true);
        assert!(v.paused, "paused while hidden stays paused");
    }
}
