use serde_json::{json, Value};

pub const PRESETS: [&str; 9] = [
    "ultrafast",
    "superfast",
    "veryfast",
    "faster",
    "fast",
    "medium",
    "slow",
    "slower",
    "veryslow",
];

pub fn defaults() -> Value {
    json!({"compression_mode":"limit","target_mb":10,"video_format":"mp4","audio_format":"mp3","image_format":"webp","max_height":0,"encoder":"auto","advanced_enabled":false,"rate_control":"target","video_bitrate_kbps":2500,"audio_bitrate_kbps":128,"crf":23,"scale_percent":100,"output_width":0,"output_height":0,"fps":0,"preset":"veryfast","audio_channels":0,"audio_sample_rate":0,"mute_audio":false,"image_quality":90,"image_lossless":false,"strip_metadata":true})
}

pub fn validate(input: &Value) -> Result<Value, String> {
    let values = input
        .as_object()
        .ok_or("Compression settings must be an object.")?;
    let mut output = defaults();
    for (key, value) in values {
        if output.get(key).is_none() {
            return Err(format!("Unknown compression setting: {key}"));
        }
        output[key] = value.clone();
    }
    for key in [
        "advanced_enabled",
        "mute_audio",
        "image_lossless",
        "strip_metadata",
    ] {
        if !output[key].is_boolean() {
            return Err(format!("Invalid setting: {key}"));
        }
    }
    for (key, choices) in [
        ("compression_mode", vec!["limit", "auto"]),
        ("video_format", vec!["mp4", "webm", "mkv", "mov"]),
        (
            "audio_format",
            vec!["mp3", "opus", "m4a", "aac", "ogg", "flac"],
        ),
        ("image_format", vec!["webp", "jpeg"]),
        ("encoder", vec!["auto", "software"]),
        ("rate_control", vec!["target", "bitrate", "quality"]),
        ("preset", PRESETS.to_vec()),
    ] {
        if !choices.contains(&output[key].as_str().unwrap_or("")) {
            return Err(format!("Invalid setting: {key}"));
        }
    }
    for (key, choices) in [
        ("max_height", vec![0, 480, 720, 1080, 1440, 2160]),
        ("audio_channels", vec![0, 1, 2]),
        ("audio_sample_rate", vec![0, 22050, 32000, 44100, 48000]),
    ] {
        if !output[key]
            .as_i64()
            .map(|n| choices.contains(&n))
            .unwrap_or(false)
        {
            return Err(format!("Invalid setting: {key}"));
        }
    }
    for (key, min, max, integer) in [
        ("target_mb", 0.000001, 1_000_000.0, false),
        ("video_bitrate_kbps", 12.0, 100_000.0, true),
        ("audio_bitrate_kbps", 8.0, 320.0, true),
        ("crf", 0.0, 51.0, true),
        ("scale_percent", 10.0, 200.0, false),
        ("output_width", 0.0, 7680.0, true),
        ("output_height", 0.0, 7680.0, true),
        ("fps", 0.0, 120.0, false),
        ("image_quality", 1.0, 100.0, true),
    ] {
        let n = output[key]
            .as_f64()
            .or_else(|| output[key].as_str().and_then(|s| s.parse().ok()))
            .ok_or_else(|| format!("Invalid setting: {key}"))?;
        if !n.is_finite() || n < min || n > max {
            return Err(format!("Invalid setting: {key}"));
        }
        if integer && n.fract() != 0.0 {
            return Err(format!("Setting requires a whole number: {key}"));
        }
        output[key] = if integer { json!(n as i64) } else { json!(n) };
    }
    if output["output_width"] == 1 || output["output_height"] == 1 {
        return Err("Dimensions must be zero or at least two pixels.".into());
    }
    Ok(output)
}

pub fn effective(input: &Value) -> Result<Value, String> {
    let mut output = validate(input)?;
    if output["advanced_enabled"] == false {
        for (key, value) in defaults().as_object().unwrap() {
            if ![
                "compression_mode",
                "target_mb",
                "video_format",
                "audio_format",
                "image_format",
                "max_height",
                "encoder",
            ]
            .contains(&key.as_str())
            {
                output[key] = value.clone();
            }
        }
    }
    if output["rate_control"] == "quality" {
        output["encoder"] = json!("software");
    }
    if output["compression_mode"] == "auto" {
        output["advanced_enabled"] = json!(true);
        output["rate_control"] = json!("quality");
        output["encoder"] = json!("software");
        output["audio_bitrate_kbps"] = json!(320);
        output["max_height"] = json!(0);
        output["scale_percent"] = json!(100);
        output["output_width"] = json!(0);
        output["output_height"] = json!(0);
        output["fps"] = json!(0);
        output["mute_audio"] = json!(false);
        output["image_quality"] = json!(100);
        output["image_lossless"] = json!(output["image_format"] == "webp");
        output["strip_metadata"] = json!(false);
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_invalid_values() {
        for value in [
            json!({"target_mb":true}),
            json!({"fps":121}),
            json!({"crf":1.5}),
            json!({"output_width":1}),
            json!({"unknown":3}),
        ] {
            assert!(validate(&value).is_err());
        }
    }
    #[test]
    fn disabled_advanced_uses_defaults() {
        let result = effective(&json!({"scale_percent":50,"rate_control":"quality"})).unwrap();
        assert_eq!(result["scale_percent"], 100);
        assert_eq!(result["rate_control"], "target");
    }
    #[test]
    fn quality_requires_software() {
        assert_eq!(
            effective(&json!({"advanced_enabled":true,"rate_control":"quality"})).unwrap()
                ["encoder"],
            "software"
        );
    }
    #[test]
    fn auto_mode_ignores_size_and_visual_degradation_controls() {
        let options = effective(&json!({"compression_mode":"auto","target_mb":1,"video_format":"mp4","audio_format":"opus","image_format":"jpeg","advanced_enabled":true,"rate_control":"bitrate","video_bitrate_kbps":100,"scale_percent":50,"max_height":480,"fps":15,"encoder":"auto","mute_audio":true,"image_lossless":false})).unwrap();
        assert_eq!(options["compression_mode"], "auto");
        assert_eq!(options["video_format"], "mp4");
        assert_eq!(options["audio_format"], "opus");
        assert_eq!(options["image_format"], "jpeg");
        assert_eq!(options["rate_control"], "quality");
        assert_eq!(options["scale_percent"], 100);
        assert_eq!(options["max_height"], 0);
        assert_eq!(options["fps"], 0);
        assert_eq!(options["encoder"], "software");
        assert_eq!(options["mute_audio"], false);
        assert_eq!(options["audio_bitrate_kbps"], 320);
        assert_eq!(options["image_quality"], 100);
        assert_eq!(options["image_lossless"], false);
    }
}
