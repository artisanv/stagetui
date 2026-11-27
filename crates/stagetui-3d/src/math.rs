//! Small 3D math: vectors and 4×4 matrices (row-major: `m[row][col]`).

use std::ops::{Add, Mul, Neg, Sub};

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct V3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

pub const fn v3(x: f32, y: f32, z: f32) -> V3 {
    V3 { x, y, z }
}

impl V3 {
    pub fn dot(self, o: V3) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: V3) -> V3 {
        v3(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }

    /// Unit length (or zero, for a zero vector).
    pub fn norm(self) -> V3 {
        let l = self.len();
        if l > 1e-20 {
            self * (1.0 / l)
        } else {
            V3::default()
        }
    }

    pub fn min(self, o: V3) -> V3 {
        v3(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: V3) -> V3 {
        v3(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for V3 {
    type Output = V3;
    fn add(self, o: V3) -> V3 {
        v3(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for V3 {
    type Output = V3;
    fn sub(self, o: V3) -> V3 {
        v3(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for V3 {
    type Output = V3;
    fn mul(self, s: f32) -> V3 {
        v3(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for V3 {
    type Output = V3;
    fn neg(self) -> V3 {
        v3(-self.x, -self.y, -self.z)
    }
}

pub type M4 = [[f32; 4]; 4];

pub const IDENTITY: M4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

pub fn mul(a: &M4, b: &M4) -> M4 {
    let mut m = [[0.0; 4]; 4];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    m
}

pub fn translate(t: V3) -> M4 {
    let mut m = IDENTITY;
    m[0][3] = t.x;
    m[1][3] = t.y;
    m[2][3] = t.z;
    m
}

pub fn scale(s: V3) -> M4 {
    let mut m = IDENTITY;
    m[0][0] = s.x;
    m[1][1] = s.y;
    m[2][2] = s.z;
    m
}

/// A rotation from a unit quaternion `[x, y, z, w]` (glTF order).
pub fn rotate_quat(q: [f32; 4]) -> M4 {
    let [x, y, z, w] = q;
    [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
            0.0,
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
            0.0,
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
            0.0,
        ],
        [0.0, 0.0, 0.0, 1.0],
    ]
}

/// A glTF matrix (16 numbers, column by column).
pub fn from_columns(a: &[f32]) -> M4 {
    let mut m = IDENTITY;
    for (i, &v) in a.iter().take(16).enumerate() {
        m[i % 4][i / 4] = v;
    }
    m
}

pub fn point(m: &M4, p: V3) -> V3 {
    v3(
        m[0][0] * p.x + m[0][1] * p.y + m[0][2] * p.z + m[0][3],
        m[1][0] * p.x + m[1][1] * p.y + m[1][2] * p.z + m[1][3],
        m[2][0] * p.x + m[2][1] * p.y + m[2][2] * p.z + m[2][3],
    )
}

/// Transform a normal: the inverse-transpose of the 3×3 part (right even with uneven scaling).
pub fn normal_matrix(m: &M4) -> [[f32; 3]; 3] {
    let a = |r: usize, c: usize| m[r][c];
    let det = a(0, 0) * (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1))
        - a(0, 1) * (a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0))
        + a(0, 2) * (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0));
    let inv = if det.abs() > 1e-20 { 1.0 / det } else { 0.0 };
    // The inverse-transpose is the cofactor matrix divided by the determinant.
    [
        [
            (a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1)) * inv,
            -(a(1, 0) * a(2, 2) - a(1, 2) * a(2, 0)) * inv,
            (a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0)) * inv,
        ],
        [
            -(a(0, 1) * a(2, 2) - a(0, 2) * a(2, 1)) * inv,
            (a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0)) * inv,
            -(a(0, 0) * a(2, 1) - a(0, 1) * a(2, 0)) * inv,
        ],
        [
            (a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1)) * inv,
            -(a(0, 0) * a(1, 2) - a(0, 2) * a(1, 0)) * inv,
            (a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0)) * inv,
        ],
    ]
}

pub fn apply3(m: &[[f32; 3]; 3], v: V3) -> V3 {
    v3(
        m[0][0] * v.x + m[0][1] * v.y + m[0][2] * v.z,
        m[1][0] * v.x + m[1][1] * v.y + m[1][2] * v.z,
        m[2][0] * v.x + m[2][1] * v.y + m[2][2] * v.z,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: V3, b: V3) -> bool {
        (a - b).len() < 1e-4
    }

    #[test]
    fn matrices_compose() {
        let m = mul(&translate(v3(1.0, 2.0, 3.0)), &scale(v3(2.0, 2.0, 2.0)));
        assert!(close(point(&m, v3(1.0, 1.0, 1.0)), v3(3.0, 4.0, 5.0)));
        // 90° about Y: x → -z.
        let h = std::f32::consts::FRAC_1_SQRT_2;
        let r = rotate_quat([0.0, h, 0.0, h]);
        assert!(close(point(&r, v3(1.0, 0.0, 0.0)), v3(0.0, 0.0, -1.0)));
        let cols: Vec<f32> = (0..16)
            .map(|i| {
                [
                    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 5.0, 6.0, 7.0, 1.0,
                ][i]
            })
            .collect();
        assert!(close(
            point(&from_columns(&cols), V3::default()),
            v3(5.0, 6.0, 7.0)
        ));
    }

    #[test]
    fn normals_stay_perpendicular_under_uneven_scale() {
        let m = scale(v3(4.0, 1.0, 1.0));
        // A surface along the diagonal x = y: its normal is (1, -1, 0).
        let n = apply3(&normal_matrix(&m), v3(1.0, -1.0, 0.0)).norm();
        let along = point(&m, v3(1.0, 1.0, 0.0));
        assert!(n.dot(along).abs() < 1e-5, "{n:?}");
        assert!((v3(3.0, 4.0, 0.0).len() - 5.0).abs() < 1e-6);
        assert!(close(
            v3(1.0, 0.0, 0.0).cross(v3(0.0, 1.0, 0.0)),
            v3(0.0, 0.0, 1.0)
        ));
    }
}
