//! The viewer's WGSL modules, composed once so the pipelines and the shader checks see the
//! same source. Every module must also compile with FXC (D3D12 on Windows, shader model 5.1):
//! the `fxc` tests below translate each entry point to HLSL as wgpu's DX12 backend does and
//! reject the patterns FXC refuses.

use std::borrow::Cow;

const COMMON: &str = include_str!("shaders/common.wgsl");

/// The hero and Hideout scene module (`viewer main`, `viewer outline`, `viewer rim depth`,
/// `viewer shadow`, `viewer fog`).
pub fn scene() -> Cow<'static, str> {
    Cow::Owned(format!("{COMMON}\n{}", include_str!("shaders/scene.wgsl")))
}

/// The studio floor's contact shadow.
pub fn floor() -> Cow<'static, str> {
    Cow::Owned(format!("{COMMON}\n{}", include_str!("shaders/floor.wgsl")))
}

/// Post, bloom threshold, down and up.
pub fn post() -> Cow<'static, str> {
    Cow::Borrowed(include_str!("shaders/post.wgsl"))
}

/// Particle spritecards.
pub fn particles() -> Cow<'static, str> {
    Cow::Borrowed(include_str!("particles/sprite.wgsl"))
}

/// A module's label, its WGSL and the (vertex, fragment) entry point pairs the viewer builds
/// pipelines from (`None` for a depth-only pipeline).
pub type Module<'a> = (&'a str, Cow<'a, str>, Vec<(&'a str, Option<&'a str>)>);

/// Every module the viewer compiles.
pub fn modules() -> Vec<Module<'static>> {
    vec![
        ("viewer scene", scene(), vec![("vs_main", Some("fs_main")), ("vs_outline", Some("fs_outline")), ("vs_depth", None), ("vs_fog", Some("fs_fog"))]),
        ("viewer floor", floor(), vec![("vs_floor", Some("fs_floor"))]),
        ("viewer post", post(), vec![("vs_full", Some("fs_post")), ("vs_full", Some("fs_threshold")), ("vs_full", Some("fs_down")), ("vs_full", Some("fs_up"))]),
        ("particles", particles(), vec![("vs_sprite", Some("fs_sprite"))]),
    ]
}

#[cfg(test)]
mod fxc;
