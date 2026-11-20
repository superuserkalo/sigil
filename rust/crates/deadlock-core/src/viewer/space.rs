//! Source 2 space to glTF space.
//!
//! Source 2 is Z up, X forward, Y left, in inches. glTF is Y up, meters. morphic wraps every
//! exported model in the same conversion VRF uses (`TRANSFORMSOURCETOGLTF`), which works out to
//! `gltf = 0.0254 * (src.y, src.z, src.x)`: Source forward (+X) becomes glTF +Z, Source up
//! (+Z) becomes glTF +Y. A hero glb therefore stands on y = 0 facing +Z.
//!
//! Scene placements are Source matrices; [`to_gltf`] turns one into `S * M * S^-1`, the matrix
//! a renderer multiplies a glb (already in glTF space) by. Matrices here are 4x4 column major,
//! the order glTF and the scene file use.

pub type Mat4 = [f64; 16];

/// Inches to meters.
pub const METERS_PER_INCH: f64 = 0.0254;

pub const IDENTITY: Mat4 = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

const S: f64 = METERS_PER_INCH;
const SOURCE_TO_GLTF: Mat4 = [
    0.0, 0.0, S, 0.0, S, 0.0, 0.0, 0.0, 0.0, S, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
/// glTF space (meters, Y up) to Source space (inches, Z up), the inverse of the export wrap.
pub const GLTF_TO_SOURCE: Mat4 = [
    0.0,
    1.0 / S,
    0.0,
    0.0,
    0.0,
    0.0,
    1.0 / S,
    0.0,
    1.0 / S,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    0.0,
    1.0,
];

/// `a * b`, column major.
pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [0.0; 16];
    for column in 0..4 {
        for row in 0..4 {
            out[column * 4 + row] = (0..4).map(|k| a[k * 4 + row] * b[column * 4 + k]).sum();
        }
    }
    out
}

/// A Source placement as a glTF placement: `S * m * S^-1`.
pub fn to_gltf(source: &Mat4) -> Mat4 {
    mul(&mul(&SOURCE_TO_GLTF, source), &GLTF_TO_SOURCE)
}

/// A Source point (inches, Z up) in glTF space (meters, Y up).
pub fn point(source: [f64; 3]) -> [f64; 3] {
    [source[1] * S, source[2] * S, source[0] * S]
}

/// A Source direction in glTF space, unit length.
pub fn direction(source: [f64; 3]) -> [f64; 3] {
    let [x, y, z] = [source[1], source[2], source[0]];
    let length = (x * x + y * y + z * z).sqrt();
    if length > 0.0 {
        [x / length, y / length, z / length]
    } else {
        [0.0, 0.0, 1.0]
    }
}

/// A Source entity's placement: QAngle `[pitch, yaw, roll]` in degrees (yaw about Z, pitch
/// about Y, roll about X), per-axis scale, then the origin. Grimoire's `sourceEntityMatrix`.
pub fn entity_matrix(origin: [f64; 3], angles: [f64; 3], scales: [f64; 3]) -> Mat4 {
    let [pitch, yaw, roll] = angles.map(f64::to_radians);
    let (sp, sy, sr) = (pitch.sin(), yaw.sin(), roll.sin());
    let (cp, cy, cr) = (pitch.cos(), yaw.cos(), roll.cos());
    let [x, y, z] = scales;
    [
        cy * cp * x,
        sy * cp * x,
        -sp * x,
        0.0,
        (cy * sp * sr - sy * cr) * y,
        (sy * sp * sr + cy * cr) * y,
        cp * sr * y,
        0.0,
        (cy * sp * cr + sy * sr) * z,
        (sy * sp * cr - cy * sr) * z,
        cp * cr * z,
        0.0,
        origin[0],
        origin[1],
        origin[2],
        1.0,
    ]
}

/// A Source 3x4 row-major transform (`m_vTransform`) as a column-major 4x4.
pub fn rows_matrix(rows: &[[f64; 4]; 3]) -> Mat4 {
    [
        rows[0][0], rows[1][0], rows[2][0], 0.0, rows[0][1], rows[1][1], rows[2][1], 0.0,
        rows[0][2], rows[1][2], rows[2][2], 0.0, rows[0][3], rows[1][3], rows[2][3], 1.0,
    ]
}

/// The inverse of an affine placement, or None when it is singular.
pub fn inverse(m: &Mat4) -> Option<Mat4> {
    // The 3x3 part by cofactors (column major: m[column * 4 + row]).
    let a = |row: usize, column: usize| m[column * 4 + row];
    let c00 = a(1, 1) * a(2, 2) - a(1, 2) * a(2, 1);
    let c01 = a(1, 2) * a(2, 0) - a(1, 0) * a(2, 2);
    let c02 = a(1, 0) * a(2, 1) - a(1, 1) * a(2, 0);
    let det = a(0, 0) * c00 + a(0, 1) * c01 + a(0, 2) * c02;
    if det.abs() < 1e-12 || !det.is_finite() {
        return None;
    }
    let inv = 1.0 / det;
    // r[row][column] of the inverse 3x3.
    let r = [
        [
            c00 * inv,
            (a(0, 2) * a(2, 1) - a(0, 1) * a(2, 2)) * inv,
            (a(0, 1) * a(1, 2) - a(0, 2) * a(1, 1)) * inv,
        ],
        [
            c01 * inv,
            (a(0, 0) * a(2, 2) - a(0, 2) * a(2, 0)) * inv,
            (a(0, 2) * a(1, 0) - a(0, 0) * a(1, 2)) * inv,
        ],
        [
            c02 * inv,
            (a(0, 1) * a(2, 0) - a(0, 0) * a(2, 1)) * inv,
            (a(0, 0) * a(1, 1) - a(0, 1) * a(1, 0)) * inv,
        ],
    ];
    let t = [m[12], m[13], m[14]];
    let mut out = IDENTITY;
    for row in 0..3 {
        for column in 0..3 {
            out[column * 4 + row] = r[row][column];
        }
        out[12 + row] = -(0..3).map(|k| r[row][k] * t[k]).sum::<f64>();
    }
    Some(out)
}

/// `m * p` for a point.
pub fn transform_point(m: &Mat4, p: [f64; 3]) -> [f64; 3] {
    let mut out = [0.0; 3];
    for (row, value) in out.iter_mut().enumerate() {
        *value = m[row] * p[0] + m[4 + row] * p[1] + m[8 + row] * p[2] + m[12 + row];
    }
    out
}

/// The matrix that takes a glTF world point (meters) to the normalized `[0, 1]^3`
/// coordinates of a Source box: `placement` (a Source entity matrix) holding the local box
/// `min..max` (inches, Source local axes). Grimoire's `(inverse(matrix) * p - min) / (max -
/// min)` with the glTF to Source conversion folded in. The local axes stay Source's: x is
/// the entity's forward, y its left, z its up. None for an empty box or a singular placement.
pub fn box_world_to_local(placement: &Mat4, min: [f64; 3], max: [f64; 3]) -> Option<Mat4> {
    let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];
    if size.iter().any(|value| *value <= 0.0 || !value.is_finite()) {
        return None;
    }
    let mut normalize = IDENTITY;
    for axis in 0..3 {
        normalize[axis * 5] = 1.0 / size[axis];
        normalize[12 + axis] = -min[axis] / size[axis];
    }
    Some(mul(&mul(&normalize, &inverse(placement)?), &GLTF_TO_SOURCE))
}

/// Column `index` (0 forward +X, 1 left +Y, 2 up +Z) of a Source placement, as a unit glTF
/// direction.
pub fn axis(source: &Mat4, index: usize) -> [f64; 3] {
    let column = index.min(2) * 4;
    direction([source[column], source[column + 1], source[column + 2]])
}

/// The forward (+X) axis of a Source placement.
pub fn forward(source: &Mat4) -> [f64; 3] {
    [source[0], source[1], source[2]]
}

/// Round to f32 for the files: the renderer works in f32 anyway.
pub fn to_f32(matrix: &Mat4) -> [f32; 16] {
    matrix.map(|value| value as f32)
}
