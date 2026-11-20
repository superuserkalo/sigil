//! The master `post_processing_volume`: fixed exposure, and its `.vpost` tone curve, bloom
//! and color correction volume. Ported from Grimoire's `hideoutScene.ts` (MIT):
//! `fixedMapExposure`, `parseMapTonemap`, `parseMapPost` and the LUT write.

use serde_json::Value as Json;

use super::super::kv::{array, flag, num, text};
use super::{HideoutBloom, HideoutTonemap};

/// The exposure a master post volume pins, or None when it adapts (auto exposure).
/// Grimoire's `fixedMapExposure`: exposure off means `2^compensation`; on, it is fixed only
/// when its min and max agree.
pub(crate) fn fixed_exposure(entity: &serde_json::Map<String, Json>) -> Option<f32> {
    let compensation = 2f64.powf(num(entity.get("exposurecompensation")).unwrap_or(0.0));
    if flag(entity.get("enableexposure")) == Some(false) {
        return Some(compensation as f32);
    }
    let bound = |linear: &str, log: &str, missing: f64| {
        num(entity.get(linear))
            .filter(|value| *value > 0.0)
            .unwrap_or_else(|| 2f64.powf(num(entity.get(log)).unwrap_or(missing)))
    };
    let min = bound("minexposure", "minlogexposure", f64::NEG_INFINITY);
    let max = bound("maxexposure", "maxlogexposure", f64::INFINITY);
    (min == max && min > 0.0 && min.is_finite()).then_some((min * compensation) as f32)
}

/// `m_toneMapParams` when `m_bHasTonemapParams` is set and every field is valid. The white
/// point stays in stops, as stored.
pub(crate) fn tonemap(post: &Json) -> Option<HideoutTonemap> {
    if flag(post.get("m_bHasTonemapParams")) != Some(true) {
        return None;
    }
    let params = post.get("m_toneMapParams")?;
    let field = |key: &str| num(params.get(key)).map(|value| value as f32);
    let non_negative = |key: &str| field(key).filter(|value| *value >= 0.0);
    Some(HideoutTonemap {
        exposure_bias: field("m_flExposureBias")?,
        shoulder_strength: non_negative("m_flShoulderStrength")?,
        linear_strength: non_negative("m_flLinearStrength")?,
        linear_angle: non_negative("m_flLinearAngle")?,
        toe_strength: non_negative("m_flToeStrength")?,
        toe_num: non_negative("m_flToeNum")?,
        toe_denom: field("m_flToeDenom").filter(|value| *value > 0.0)?,
        white_point: field("m_flWhitePoint")?,
    })
}

/// `m_bloomParams` when `m_bHasBloomParams` is set and every field is valid.
pub(crate) fn bloom(post: &Json) -> Option<HideoutBloom> {
    if flag(post.get("m_bHasBloomParams")) != Some(true) {
        return None;
    }
    let params = post.get("m_bloomParams")?;
    let field = |key: &str| num(params.get(key)).map(|value| value as f32);
    let blend_mode = match text(params.get("m_blendMode"))? {
        "BLOOM_BLEND_ADD" => "add",
        "BLOOM_BLEND_SCREEN" => "screen",
        "BLOOM_BLEND_BLUR" => "blur",
        _ => return None,
    };
    let weights = array(params.get("m_flBlurWeight"));
    let tints = array(params.get("m_vBlurTint"));
    if weights.len() != 5 || tints.len() != 5 {
        return None;
    }
    let mut blur_weights = [0.0f32; 5];
    let mut blur_tints = [[0.0f32; 3]; 5];
    for (index, (weight, color)) in blur_weights.iter_mut().zip(&mut blur_tints).enumerate() {
        *weight = num(weights.get(index))? as f32;
        let tint = array(tints.get(index));
        if tint.len() != 3 {
            return None;
        }
        for (channel, value) in color.iter_mut().enumerate() {
            *value = num(tint.get(channel))? as f32;
        }
    }
    Some(HideoutBloom {
        blend_mode: blend_mode.into(),
        bloom_strength: field("m_flBloomStrength")?,
        screen_bloom_strength: field("m_flScreenBloomStrength")?,
        blur_bloom_strength: field("m_flBlurBloomStrength")?,
        threshold: field("m_flBloomThreshold")?,
        threshold_width: field("m_flBloomThresholdWidth")?,
        blur_weights,
        blur_tints,
    })
}

/// The color correction volume as stored: `dimension` and `dimension^3` RGBA8 texels, x
/// fastest. None when the vpost has none (dimension 0 or 1); an error when its size is wrong.
pub(crate) fn color_correction(post: &Json) -> Result<Option<(u32, Vec<u8>)>, String> {
    let Some(dimension) = num(post.get("m_nColorCorrectionVolumeDim")) else {
        return Ok(None);
    };
    if !(0.0..=64.0).contains(&dimension) || dimension.fract() != 0.0 {
        return Err(format!("color correction dimension {dimension}"));
    }
    let dimension = dimension as u32;
    if dimension <= 1 {
        return Ok(None);
    }
    let data = array(post.get("m_colorCorrectionVolumeData"));
    let expected = (dimension as usize).pow(3) * 4;
    if data.len() != expected {
        return Err(format!(
            "color correction holds {} bytes, {expected} expected",
            data.len()
        ));
    }
    let bytes = data
        .iter()
        .map(|value| {
            value
                .as_u64()
                .filter(|byte| *byte <= 255)
                .map(|byte| byte as u8)
        })
        .collect::<Option<Vec<u8>>>()
        .ok_or("color correction byte out of range")?;
    Ok(Some((dimension, bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entity(json: serde_json::Value) -> serde_json::Map<String, Json> {
        json.as_object().unwrap().clone()
    }

    fn vpost() -> Json {
        serde_json::json!({
            "m_bHasTonemapParams": true,
            "m_toneMapParams": {
                "m_flExposureBias": 0.8992, "m_flShoulderStrength": 0.3538, "m_flLinearStrength": 0.3258,
                "m_flLinearAngle": 0.2528, "m_flToeStrength": 0.6966, "m_flToeNum": 0.0,
                "m_flToeDenom": 0.7819, "m_flWhitePoint": 3.9996,
            },
            "m_bHasBloomParams": true,
            "m_bloomParams": {
                "m_blendMode": "BLOOM_BLEND_SCREEN", "m_flBloomStrength": 0.5, "m_flScreenBloomStrength": 0.25,
                "m_flBlurBloomStrength": 0.0, "m_flBloomThreshold": 1.0, "m_flBloomThresholdWidth": 0.999,
                "m_flBlurWeight": [0.1, 0.2, 0.3, 0.2, 0.2],
                "m_vBlurTint": [[1, 1, 1], [1, 0.5, 0], [1, 1, 1], [0, 0, 1], [1, 1, 1]],
            },
            "m_nColorCorrectionVolumeDim": 2,
            "m_colorCorrectionVolumeData": (0..32).collect::<Vec<u32>>(),
        })
    }

    #[test]
    fn viewer_vpost_tonemap_bloom_and_lut_parse() {
        let post = vpost();
        let tone = tonemap(&post).unwrap();
        assert_eq!(tone.exposure_bias, 0.8992);
        assert_eq!(tone.toe_num, 0.0);
        assert_eq!(tone.toe_denom, 0.7819);
        assert_eq!(tone.white_point, 3.9996);
        let bloom = bloom(&post).unwrap();
        assert_eq!(bloom.blend_mode, "screen");
        assert_eq!(bloom.screen_bloom_strength, 0.25);
        assert_eq!(bloom.threshold_width, 0.999);
        assert_eq!(bloom.blur_weights[2], 0.3);
        assert_eq!(bloom.blur_tints[1], [1.0, 0.5, 0.0]);
        let (dimension, bytes) = color_correction(&post).unwrap().unwrap();
        assert_eq!(dimension, 2);
        assert_eq!(bytes, (0..32).collect::<Vec<u8>>());

        // JSON field names are the contract's.
        let json = serde_json::to_value(&tone).unwrap();
        for key in [
            "exposureBias",
            "shoulderStrength",
            "linearStrength",
            "linearAngle",
            "toeStrength",
            "toeNum",
            "toeDenom",
            "whitePoint",
        ] {
            assert!(json.get(key).is_some(), "{key}");
        }
        let json = serde_json::to_value(super::bloom(&post).unwrap()).unwrap();
        for key in [
            "blendMode",
            "bloomStrength",
            "screenBloomStrength",
            "blurBloomStrength",
            "threshold",
            "thresholdWidth",
            "blurWeights",
            "blurTints",
        ] {
            assert!(json.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn viewer_vpost_without_params_is_none() {
        let mut post = vpost();
        post["m_bHasTonemapParams"] = serde_json::json!(false);
        post["m_bHasBloomParams"] = serde_json::json!(false);
        post["m_nColorCorrectionVolumeDim"] = serde_json::json!(0);
        assert!(tonemap(&post).is_none());
        assert!(bloom(&post).is_none());
        assert_eq!(color_correction(&post), Ok(None));
        assert!(tonemap(&serde_json::json!({})).is_none());

        let mut short = vpost();
        short["m_colorCorrectionVolumeData"] = serde_json::json!([1, 2, 3]);
        assert!(color_correction(&short).is_err());
        let mut odd = vpost();
        odd["m_bloomParams"]["m_blendMode"] = serde_json::json!("BLOOM_BLEND_UNKNOWN");
        assert!(bloom(&odd).is_none());
    }

    #[test]
    fn viewer_fixed_exposure_rules() {
        // Exposure off: the compensation alone.
        let off = entity(serde_json::json!({ "enableexposure": false, "exposurecompensation": 1 }));
        assert_eq!(fixed_exposure(&off), Some(2.0));
        // Min equal to max pins it.
        let pinned = entity(
            serde_json::json!({ "minexposure": 0.5, "maxexposure": 0.5, "exposurecompensation": -1 }),
        );
        assert_eq!(fixed_exposure(&pinned), Some(0.25));
        // Log bounds when the linear ones are zero.
        let log = entity(
            serde_json::json!({ "minexposure": 0, "minlogexposure": 1, "maxlogexposure": 1 }),
        );
        assert_eq!(fixed_exposure(&log), Some(2.0));
        // A range adapts: auto exposure.
        let range = entity(serde_json::json!({ "minexposure": 0.25, "maxexposure": 4.0 }));
        assert_eq!(fixed_exposure(&range), None);
        // No bounds at all: auto.
        assert_eq!(
            fixed_exposure(&entity(serde_json::json!({ "master": true }))),
            None
        );
    }
}
