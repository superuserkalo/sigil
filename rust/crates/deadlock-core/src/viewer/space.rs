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
const GLTF_TO_SOURCE: Mat4 = [
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

/// The forward (+X) axis of a Source placement.
pub fn forward(source: &Mat4) -> [f64; 3] {
    [source[0], source[1], source[2]]
}

/// Round to f32 for the files: the renderer works in f32 anyway.
pub fn to_f32(matrix: &Mat4) -> [f32; 16] {
    matrix.map(|value| value as f32)
}
