//! `stagetui-plugin-3d`: look at 3D models (glTF, OBJ, STL) in a pane and turn them
//! with the mouse. Drawn on the CPU (every core), so it needs no graphics card.

mod gltf;
mod math;
mod model;
mod obj;
mod render;
mod stl;

use std::f32::consts::{FRAC_PI_2, PI};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stagetui_plugin_api::{Describe, Frame, Mouse, PaneSource, Ready, Start, serve};

use crate::math::V3;
use crate::model::{Model, Up};
use crate::render::{Camera, FOV_Y, Look};

fn main() {
    std::process::exit(serve(Viewer::default()));
}

const START_YAW: f32 = 0.6;
const START_PITCH: f32 = 0.35;
/// Turning speed while spinning (radians a second).
const SPIN: f32 = 0.5;
/// While spinning, a new picture this often (20 a second).
const SPIN_EVERY: Duration = Duration::from_millis(50);
/// After this long without moving, the picture is drawn once more with smooth edges.
const SETTLE: Duration = Duration::from_millis(150);

struct Viewer {
    model: Option<Model>,
    edges: Option<Vec<[u32; 2]>>,
    path: PathBuf,
    format: &'static str,
    up: Option<Up>,
    cam: Camera,
    look: Look,
    spin: bool,
    size: (u32, u32),
    /// The picture must be drawn again.
    dirty: bool,
    /// The last picture was quick (no smoothing): smooth it once things stop.
    rough: bool,
    last_move: Instant,
    last_frame: Option<Duration>,
    /// How long a smooth picture takes; smoothing is kept while moving if it's quick.
    smooth_cost: Duration,
    drag: Option<(String, u32, u32)>,
    status: String,
    events: Vec<(String, Value)>,
}

impl Default for Viewer {
    fn default() -> Self {
        Viewer {
            model: None,
            edges: None,
            path: PathBuf::new(),
            format: "",
            up: None,
            cam: Camera {
                yaw: START_YAW,
                pitch: START_PITCH,
                distance: 3.0,
                target: V3::default(),
            },
            look: Look {
                bg: [26, 27, 38],
                fg: [192, 202, 245],
                accent: [187, 154, 247],
                wireframe: false,
                grid: true,
            },
            spin: true,
            size: (1, 1),
            dirty: true,
            rough: false,
            last_move: Instant::now(),
            last_frame: None,
            smooth_cost: Duration::from_millis(1),
            drag: None,
            status: String::new(),
            events: Vec::new(),
        }
    }
}

/// `#rrggbb` → bytes.
fn hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn thousands(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

impl Viewer {
    /// How far away the camera must be for the whole model to fit.
    fn fit_distance(&self) -> f32 {
        let (w, h) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
        let half_y = FOV_Y / 2.0;
        let half_x = (half_y.tan() * w / h).atan();
        0.95 / half_y.min(half_x).sin()
    }

    fn reset(&mut self) {
        self.cam = Camera {
            yaw: START_YAW,
            pitch: START_PITCH,
            distance: self.fit_distance(),
            target: V3::default(),
        };
        self.moved();
    }

    fn moved(&mut self) {
        self.dirty = true;
        self.last_move = Instant::now();
    }

    fn open(&mut self, path: &Path) -> Result<(), String> {
        let (model, format) = model::load(path, self.up)?;
        self.edges = None;
        self.events.push((
            "loaded".into(),
            json!({"path": path.display().to_string(), "triangles": model.triangles.len(), "format": format}),
        ));
        self.model = Some(model);
        self.format = format;
        self.path = path.to_path_buf();
        if self.look.wireframe {
            self.edges = self.model.as_ref().map(Model::edges);
        }
        self.reset();
        Ok(())
    }

    fn turn(&mut self, yaw: f32, pitch: f32) {
        self.cam.yaw = (self.cam.yaw + yaw).rem_euclid(2.0 * PI);
        self.cam.pitch = (self.cam.pitch + pitch).clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
        self.moved();
    }

    fn zoom(&mut self, factor: f32) {
        self.cam.distance = (self.cam.distance * factor).clamp(0.2, 60.0);
        self.moved();
    }

    fn pan(&mut self, dx: f32, dy: f32) {
        let (right, up, _) = self.cam.axes();
        // A drag across the whole pane moves the view by what the pane shows.
        let span = 2.0 * self.cam.distance * (FOV_Y / 2.0).tan();
        let k = span / self.size.1.max(1) as f32;
        self.cam.target = self.cam.target - right * (dx * k) + up * (dy * k);
        self.moved();
    }

    fn set_wireframe(&mut self, on: bool) {
        self.look.wireframe = on;
        if on && self.edges.is_none() {
            self.edges = self.model.as_ref().map(Model::edges);
        }
        self.moved();
    }

    fn state(&self) -> Value {
        let (tris, verts) = self
            .model
            .as_ref()
            .map_or((0, 0), |m| (m.triangles.len(), m.positions.len()));
        json!({
            "path": self.path.display().to_string(),
            "format": self.format,
            "triangles": tris,
            "vertices": verts,
            "yaw": self.cam.yaw.to_degrees(),
            "pitch": self.cam.pitch.to_degrees(),
            "distance": self.cam.distance,
            "spin": self.spin,
            "wireframe": self.look.wireframe,
            "grid": self.look.grid,
        })
    }

    fn picture(&self, ss: u32) -> Option<Vec<u8>> {
        let model = self.model.as_ref()?;
        Some(render::render(
            model,
            self.edges.as_deref(),
            &self.cam,
            &self.look,
            self.size.0,
            self.size.1,
            ss,
        ))
    }

    fn status_line(&self) -> String {
        let name = self
            .path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let tris = self.model.as_ref().map_or(0, |m| m.triangles.len());
        let spin = if self.spin {
            "space stop"
        } else {
            "space spin"
        };
        format!(
            "◆ {name} · {} triangles · drag turn · wheel zoom · {spin} · w wires · r reset",
            thousands(tris)
        )
    }
}

fn number(v: &Value, key: &str) -> Option<f32> {
    v.get(key).and_then(Value::as_f64).map(|f| f as f32)
}

impl PaneSource for Viewer {
    fn describe(&self) -> Describe {
        let mut d = Describe::new("3d", "3D", "cube");
        d.version = env!("CARGO_PKG_VERSION").into();
        d
    }

    fn start(&mut self, start: &Start) -> Result<Ready, String> {
        let args = &start.args;
        self.size = (start.width.max(1), start.height.max(1));
        if let Some(c) = hex(&start.theme.bg) {
            self.look.bg = c;
        }
        if let Some(c) = hex(&start.theme.fg) {
            self.look.fg = c;
        }
        if let Some(c) = hex(&start.theme.accent) {
            self.look.accent = c;
        }
        self.up = match args.get("up").and_then(Value::as_str) {
            Some("z" | "Z") => Some(Up::Z),
            Some("y" | "Y") => Some(Up::Y),
            _ => None,
        };
        self.spin = args.get("spin").and_then(Value::as_bool).unwrap_or(true);
        self.look.grid = args.get("grid").and_then(Value::as_bool).unwrap_or(true);
        let path = ["path", "src", "model"]
            .iter()
            .find_map(|k| args.get(*k).and_then(Value::as_str))
            .ok_or("no model given: StagePane(\"3d\", path=\"model.glb\")")?
            .to_string();
        self.open(Path::new(&path))?;
        self.set_wireframe(
            args.get("wireframe")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        );
        Ok(Ready {
            keys: [
                "left", "right", "up", "down", "+", "=", "-", "space", "r", "home", "w", "g", "f",
            ]
            .map(String::from)
            .to_vec(),
            text: false,
            fps: 30,
        })
    }

    fn resize(&mut self, width: u32, height: u32) {
        let old = self.fit_distance();
        self.size = (width.max(1), height.max(1));
        // Keep the same framing.
        self.cam.distance *= self.fit_distance() / old;
        self.moved();
    }

    fn frame(&mut self, now: Duration) -> Option<Frame> {
        let dt = self
            .last_frame
            .map_or(Duration::ZERO, |t| now.saturating_sub(t));
        self.last_frame = Some(now);
        if self.spin && self.drag.is_none() && self.model.is_some() {
            // Spinning is for looking, not for speed: fewer pictures, less work.
            if dt < SPIN_EVERY && !self.dirty {
                self.last_frame = Some(now - dt);
                return None;
            }
            self.cam.yaw = (self.cam.yaw + SPIN * dt.as_secs_f32().min(0.1)).rem_euclid(2.0 * PI);
            self.dirty = true;
        }
        let settled = !self.spin && self.last_move.elapsed() >= SETTLE;
        // Smooth pictures while moving only when they are cheap.
        let quick = self.smooth_cost <= Duration::from_millis(12);
        let ss = if self.dirty {
            if quick || settled { 2 } else { 1 }
        } else if self.rough && settled {
            2
        } else {
            return None;
        };
        let began = Instant::now();
        let rgba = self.picture(ss)?;
        if ss == 2 {
            self.smooth_cost = began.elapsed();
        } else {
            // A quick picture costs about a quarter of a smooth one.
            self.smooth_cost = self.smooth_cost.min(began.elapsed() * 4);
        }
        self.dirty = false;
        self.rough = ss == 1;
        Some(Frame {
            width: self.size.0,
            height: self.size.1,
            rgba,
        })
    }

    fn key(&mut self, key: &str, _text: Option<&str>) -> bool {
        let step = 15f32.to_radians();
        match key {
            "left" => self.turn(step, 0.0),
            "right" => self.turn(-step, 0.0),
            "up" => self.turn(0.0, step),
            "down" => self.turn(0.0, -step),
            "+" | "=" => self.zoom(0.85),
            "-" => self.zoom(1.0 / 0.85),
            "space" => {
                self.spin = !self.spin;
                self.moved();
            }
            "r" | "home" => self.reset(),
            "w" => self.set_wireframe(!self.look.wireframe),
            "g" => {
                self.look.grid = !self.look.grid;
                self.moved();
            }
            "f" => {
                // Frame the model again, keeping the angle.
                self.cam.distance = self.fit_distance();
                self.cam.target = V3::default();
                self.moved();
            }
            _ => return false,
        }
        true
    }

    fn mouse(&mut self, m: &Mouse) {
        let button = m.button.clone().unwrap_or_else(|| "left".into());
        match m.kind.as_str() {
            "down" => {
                self.drag = Some((button, m.x, m.y));
                self.spin = false;
                self.moved();
            }
            "drag" => {
                let Some((b, x, y)) = self.drag.clone() else {
                    self.drag = Some((button, m.x, m.y));
                    return;
                };
                let (dx, dy) = (m.x as f32 - x as f32, m.y as f32 - y as f32);
                if b == "left" {
                    // Across the whole pane: one full turn.
                    let (w, h) = (self.size.0.max(1) as f32, self.size.1.max(1) as f32);
                    self.turn(-dx / w * 2.0 * PI, dy / h * PI);
                } else {
                    self.pan(dx, dy);
                }
                self.drag = Some((b, m.x, m.y));
            }
            "up" => {
                self.drag = None;
                self.moved();
            }
            "scroll_up" => self.zoom(0.88),
            "scroll_down" => self.zoom(1.0 / 0.88),
            _ => {}
        }
    }

    fn call(&mut self, method: &str, args: &Value) -> Result<Value, String> {
        let flag = |current: bool| {
            args.as_bool()
                .or_else(|| args.get("on").and_then(Value::as_bool))
                .unwrap_or(!current)
        };
        match method {
            "load" | "open" => {
                let path = args
                    .as_str()
                    .or_else(|| args.get("path").and_then(Value::as_str))
                    .ok_or("load needs a file, e.g. call(\"load\", \"model.glb\")")?;
                self.open(Path::new(path))?;
            }
            "reset" => self.reset(),
            "view" => {
                if let Some(y) = number(args, "yaw") {
                    self.cam.yaw = y.to_radians().rem_euclid(2.0 * PI);
                }
                if let Some(p) = number(args, "pitch") {
                    self.cam.pitch = p.to_radians().clamp(-FRAC_PI_2 + 0.01, FRAC_PI_2 - 0.01);
                }
                if let Some(d) = number(args, "distance") {
                    self.cam.distance = d.clamp(0.2, 60.0);
                }
                self.moved();
            }
            "spin" => {
                self.spin = flag(self.spin);
                self.moved();
            }
            "wireframe" => self.set_wireframe(flag(self.look.wireframe)),
            "grid" => {
                self.look.grid = flag(self.look.grid);
                self.moved();
            }
            "save" => {
                let path = args
                    .as_str()
                    .or_else(|| args.get("path").and_then(Value::as_str))
                    .ok_or("save needs a file name, e.g. call(\"save\", \"view.png\")")?;
                let rgba = self.picture(2).ok_or("no model is open")?;
                image::RgbaImage::from_raw(self.size.0, self.size.1, rgba)
                    .ok_or("the picture has the wrong size")?
                    .save(path)
                    .map_err(|e| format!("couldn't save \"{path}\": {e}"))?;
                return Ok(json!(path));
            }
            "state" => {}
            other => return Err(format!("the 3D add-on has no method \"{other}\"")),
        }
        Ok(self.state())
    }

    fn status(&mut self) -> Option<String> {
        let line = self.status_line();
        (line != self.status).then(|| {
            self.status = line.clone();
            line
        })
    }

    fn events(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn started(path: &Path) -> Viewer {
        let mut v = Viewer::default();
        v.start(&Start {
            width: 160,
            height: 96,
            cols: 10,
            rows: 3,
            args: json!({"path": path.display().to_string()}),
            theme: Default::default(),
            protocol: 2,
        })
        .unwrap();
        v
    }

    fn cube_file(test: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("stagetui-3d-{test}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("cube.obj");
        std::fs::write(
            &f,
            "v -1 -1 -1\nv 1 -1 -1\nv 1 1 -1\nv -1 1 -1\nv -1 -1 1\nv 1 -1 1\nv 1 1 1\nv -1 1 1\n\
             f 1 4 3 2\nf 5 6 7 8\nf 1 2 6 5\nf 3 4 8 7\nf 2 3 7 6\nf 1 5 8 4\n",
        )
        .unwrap();
        f
    }

    #[test]
    fn starts_draws_turns_and_zooms() {
        let f = cube_file("draw");
        let mut v = started(&f);
        assert_eq!(v.events()[0].0, "loaded");
        let first = v.frame(Duration::ZERO).expect("a first picture");
        assert_eq!(first.rgba.len(), 160 * 96 * 4);
        // Spinning: another picture comes.
        assert!(v.frame(Duration::from_millis(50)).is_some());
        // Stop spinning, then nothing new until something moves.
        v.key("space", None);
        let _ = v.frame(Duration::from_millis(100));
        v.last_move = Instant::now() - Duration::from_secs(1);
        let _ = v.frame(Duration::from_millis(150)); // the smooth one, if the last was quick
        assert!(v.frame(Duration::from_millis(200)).is_none());
        let yaw = v.state()["yaw"].as_f64().unwrap();
        v.mouse(&Mouse {
            kind: "down".into(),
            button: Some("left".into()),
            x: 10,
            y: 10,
        });
        v.mouse(&Mouse {
            kind: "drag".into(),
            button: Some("left".into()),
            x: 50,
            y: 10,
        });
        assert_ne!(v.state()["yaw"].as_f64().unwrap(), yaw);
        let d = v.state()["distance"].as_f64().unwrap();
        v.mouse(&Mouse {
            kind: "scroll_up".into(),
            button: None,
            x: 0,
            y: 0,
        });
        assert!(v.state()["distance"].as_f64().unwrap() < d);
        assert!(v.frame(Duration::from_millis(250)).is_some());
        assert!(v.status().unwrap().contains("cube.obj · 12 triangles"));
        std::fs::remove_dir_all(f.parent().unwrap()).unwrap();
    }

    #[test]
    fn calls() {
        let f = cube_file("calls");
        let mut v = started(&f);
        let s = v
            .call("view", &json!({"yaw": 90, "pitch": 10, "distance": 5}))
            .unwrap();
        assert!((s["yaw"].as_f64().unwrap() - 90.0).abs() < 1e-3);
        assert_eq!(s["distance"], json!(5.0));
        assert_eq!(
            v.call("wireframe", &json!(true)).unwrap()["wireframe"],
            json!(true)
        );
        assert_eq!(v.call("spin", &Value::Null).unwrap()["spin"], json!(false));
        let png = f.with_file_name("view.png");
        v.call("save", &json!(png.display().to_string())).unwrap();
        assert!(image::open(&png).unwrap().width() == 160);
        assert!(
            v.call("explode", &Value::Null)
                .unwrap_err()
                .contains("no method")
        );
        assert!(
            v.call("load", &json!("/nope/missing.glb"))
                .unwrap_err()
                .contains("not found")
        );
        std::fs::remove_dir_all(f.parent().unwrap()).unwrap();
    }

    #[test]
    fn no_model_says_how() {
        let mut v = Viewer::default();
        let e = v
            .start(&Start {
                width: 10,
                height: 10,
                cols: 1,
                rows: 1,
                args: json!({}),
                theme: Default::default(),
                protocol: 2,
            })
            .unwrap_err();
        assert!(e.contains("path="), "{e}");
        assert_eq!(thousands(1234567), "1,234,567");
        assert_eq!(hex("#ff8000"), Some([255, 128, 0]));
    }
}
