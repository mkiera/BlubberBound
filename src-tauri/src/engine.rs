use crate::settings;
use serde_json::{json, Value};
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

const IMAGES: &[&str] = &[
    "png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif", "avif",
];
const VIDEOS: &[&str] = &[
    "mp4", "mkv", "mov", "webm", "avi", "m4v", "flv", "wmv", "mpeg", "mpg", "ts", "mts", "m2ts",
    "3gp", "vob",
];
const AUDIO: &[&str] = &[
    "mp3", "wav", "flac", "aac", "m4a", "ogg", "opus", "wma", "aiff", "aif",
];
fn ext(path: &Path) -> String {
    path.extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase()
}
pub fn supported_file(path: &Path) -> bool {
    path.is_file()
        && [IMAGES, VIDEOS, AUDIO]
            .iter()
            .any(|list| list.contains(&ext(path).as_str()))
}
fn command(tool: &Path) -> Command {
    let mut cmd = Command::new(tool);
    cmd.stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd
}
fn tool(name: &str) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os(format!("BLUBBERBOUND_{}", name.to_uppercase()))
        .map(PathBuf::from)
        .filter(|p| p.is_file())
    {
        return Some(path);
    }
    let file = format!("{name}{}", if cfg!(windows) { ".exe" } else { "" });
    let mut roots = vec![];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
            roots.push(parent.join("resources"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    #[cfg(debug_assertions)]
    roots.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".."));
    for root in roots {
        for folder in ["tools", "bin", ""] {
            let p = root.join(folder).join(&file);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(&file))
            .find(|p| p.is_file())
    })
}
pub fn find_tools() -> Value {
    json!({"ffmpeg":tool("ffmpeg"),"ffprobe":tool("ffprobe")})
}
fn check(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Compression cancelled.".into())
    } else {
        Ok(())
    }
}
fn n(value: &Value) -> f64 {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .filter(|n| n.is_finite())
        .unwrap_or(0.0)
}
fn s<'a>(value: &'a Value, key: &str) -> &'a str {
    value[key].as_str().unwrap_or("")
}
fn b(value: &Value, key: &str) -> bool {
    value[key].as_bool().unwrap_or(false)
}
fn ratio(value: &Value, default: f64) -> f64 {
    let raw = value.as_str().unwrap_or("").replace(':', "/");
    let parts: Vec<_> = raw.split('/').collect();
    if parts.len() == 2 {
        let a = parts[0].parse::<f64>().unwrap_or(0.0);
        let d = parts[1].parse::<f64>().unwrap_or(0.0);
        if a / d > 0.0 && (a / d).is_finite() {
            return a / d;
        }
    }
    default
}
fn capture(
    tool: &Path,
    args: &[String],
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let mut child = command(tool)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let out = thread::spawn(move || {
        let mut result = Vec::new();
        stdout
            .take(8 * 1024 * 1024)
            .read_to_end(&mut result)
            .map(|_| result)
    });
    let err = thread::spawn(move || {
        let mut result = Vec::new();
        stderr
            .take(1024 * 1024)
            .read_to_end(&mut result)
            .map(|_| result)
    });
    let start = Instant::now();
    let status = loop {
        if cancel.load(Ordering::Relaxed) || start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            let _ = out.join();
            let _ = err.join();
            check(cancel)?;
            return Err("Reading media information timed out.".into());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e.to_string());
            }
        }
    };
    let output = out
        .join()
        .map_err(|_| "Media reader stopped.")?
        .map_err(|e| e.to_string())?;
    let errors = err
        .join()
        .map_err(|_| "Media reader stopped.")?
        .map_err(|e| e.to_string())?;
    if !status.success() {
        return Err(format!(
            "This media file could not be read: {}",
            String::from_utf8_lossy(&errors)
        ));
    }
    Ok(output)
}
pub fn probe(path: &Path) -> Result<Value, String> {
    probe_cancel(path, &AtomicBool::new(false))
}
fn probe_cancel(path: &Path, cancel: &AtomicBool) -> Result<Value, String> {
    check(cancel)?;
    let path = path.canonicalize().map_err(|e| e.to_string())?;
    if !supported_file(&path) {
        return Err("This file type is not supported.".into());
    }
    let image = IMAGES.contains(&ext(&path).as_str());
    let mut args = vec![
        "-v".into(),
        "error".into(),
        "-show_streams".into(),
        "-show_format".into(),
        "-of".into(),
        "json".into(),
    ];
    if image {
        args.extend([
            "-count_frames".into(),
            "-read_intervals".into(),
            "%+#2".into(),
        ]);
    }
    args.push(path.to_string_lossy().into_owned());
    let data: Value = serde_json::from_slice(&capture(
        &tool("ffprobe")
            .ok_or("FFprobe was not found. Put ffmpeg and ffprobe in the tools folder.")?,
        &args,
        cancel,
        Duration::from_secs(30),
    )?)
    .map_err(|e| e.to_string())?;
    let streams = data["streams"]
        .as_array()
        .ok_or("No usable media streams were found.")?;
    let video = streams
        .iter()
        .find(|v| s(v, "codec_type") == "video" && n(&v["disposition"]["attached_pic"]) == 0.0);
    let audio = streams.iter().find(|v| s(v, "codec_type") == "audio");
    if video.is_none() && audio.is_none() {
        return Err("No usable video or audio stream was found.".into());
    }
    let mut duration = n(&data["format"]["duration"]);
    if duration <= 0.0 {
        duration = streams
            .iter()
            .map(|v| n(&v["duration"]))
            .fold(0.0, f64::max);
    }
    if image {
        if video
            .map(|v| n(&v["nb_read_frames"]) > 1.0 || n(&v["nb_frames"]) > 1.0)
            .unwrap_or(false)
        {
            return Err(
                "Animated images are not supported. Export a video first to preserve every frame."
                    .into(),
            );
        }
        duration = 0.0;
    } else if duration <= 0.0 {
        return Err(
            "The media duration is unavailable. This file cannot be compressed to a size limit."
                .into(),
        );
    }
    let mut info = json!({"path":path,"name":path.file_name().unwrap_or_default().to_string_lossy(),"size":fs::metadata(&path).map_err(|e|e.to_string())?.len(),"duration":duration,"width":0,"height":0,"kind":if image {"image"}else if video.is_some(){"video"}else{"audio"},"has_audio":audio.is_some(),"audio_index":audio.map(|v|v["index"].clone()),"audio_codec":audio.map(|v|v["codec_name"].clone())});
    if let Some(v) = audio {
        info["audio_channels"] = json!(n(&v["channels"]));
        info["audio_sample_rate"] = json!(n(&v["sample_rate"]));
        info["audio_bitrate"] = json!(n(&v["bit_rate"]));
    }
    if let Some(v) = video {
        let mut width = (n(&v["width"]) * ratio(&v["sample_aspect_ratio"], 1.0)) as u32;
        let mut height = n(&v["height"]) as u32;
        let mut rotation = n(&v["tags"]["rotate"]);
        if let Some(sides) = v["side_data_list"].as_array() {
            for side in sides {
                if side.get("rotation").is_some() {
                    rotation = n(&side["rotation"]);
                }
            }
        }
        let orientation = if image {
            let (mut exif, _) = image_metadata(&path)?;
            let stored = exif_orientation(&mut exif, false);
            if stored != 1 {
                stored
            } else {
                match (rotation.round() as i64).rem_euclid(360) {
                    90 => 8,
                    180 => 3,
                    270 => 6,
                    _ => 1,
                }
            }
        } else {
            1
        };
        info["orientation"] = json!(orientation);
        if (image && orientation >= 5) || (!image && rotation.round() as i64 % 180 != 0) {
            std::mem::swap(&mut width, &mut height);
        }
        if width == 0 || height == 0 || (!image && (width < 2 || height < 2)) {
            return Err("The video dimensions are too small or invalid.".into());
        }
        info["width"] = json!(width);
        info["height"] = json!(height);
        info["fps"] = json!(ratio(&v["avg_frame_rate"], 30.0));
        info["frame_count"] = json!(n(&v["nb_frames"]));
        info["video_duration"] = json!(n(&v["duration"]));
        info["video_index"] = v["index"].clone();
        info["codec"] = v["codec_name"].clone();
        info["pixel_format"] = v["pix_fmt"].clone();
    }
    Ok(info)
}
#[derive(Clone, Copy)]
enum ProgressTotal {
    Seconds(f64),
    Frames(f64),
}
impl ProgressTotal {
    fn media(info: &Value, options: &Value, duration: f64, sample: bool) -> Self {
        if s(info, "kind") != "video" {
            return Self::Seconds(duration);
        }
        if !sample && n(&options["fps"]) == 0.0 && n(&info["frame_count"]) > 0.0 {
            return Self::Frames(n(&info["frame_count"]));
        }
        let fps = if n(&options["fps"]) > 0.0 {
            n(&options["fps"]).min(n(&info["fps"]))
        } else {
            n(&info["fps"])
        };
        let duration = if !sample && n(&info["video_duration"]) > 0.0 {
            duration.min(n(&info["video_duration"]))
        } else {
            duration
        };
        if fps > 0.0 {
            Self::Frames((fps * duration).ceil())
        } else {
            Self::Seconds(duration)
        }
    }
    fn count(self, line: &str) -> Option<u64> {
        line.strip_prefix(match self {
            Self::Seconds(_) => "out_time_us=",
            Self::Frames(_) => "frame=",
        })?
        .trim()
        .parse()
        .ok()
    }
    fn percent(self, count: u64) -> f64 {
        let total = match self {
            Self::Seconds(seconds) => seconds * 1_000_000.0,
            Self::Frames(frames) => frames,
        };
        if total > 0.0 {
            (count as f64 / total * 99.0).clamp(0.0, 99.0)
        } else {
            0.0
        }
    }
}
fn run(
    ffmpeg: &Path,
    args: &[String],
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
    total: ProgressTotal,
    stage: &str,
    timeout: Option<Duration>,
) -> Result<u64, String> {
    check(cancel)?;
    let mut child = command(ffmpeg)
        .args(["-stats_period", "0.1"])
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let completed = Arc::new(AtomicU64::new(0));
    let read_completed = completed.clone();
    let frames = Arc::new(AtomicU64::new(0));
    let read_frames = frames.clone();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(count) = line
                .strip_prefix("frame=")
                .and_then(|value| value.trim().parse().ok())
            {
                read_frames.fetch_max(count, Ordering::Relaxed);
            }
            if let Some(count) = total.count(&line) {
                read_completed.fetch_max(count, Ordering::Relaxed);
            }
        }
    });
    let errors = thread::spawn(move || {
        let mut input = stderr;
        let mut blocks = VecDeque::new();
        let mut buffer = [0; 2048];
        while let Ok(count) = input.read(&mut buffer) {
            if count == 0 {
                break;
            }
            blocks.push_back(buffer[..count].to_vec());
            if blocks.len() > 32 {
                blocks.pop_front();
            }
        }
        blocks.into_iter().flatten().collect::<Vec<_>>()
    });
    let started = Instant::now();
    let mut last = -1;
    let mut failure = None;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            failure = Some("Compression cancelled.".to_owned());
            let _ = child.kill();
            break child.wait().map_err(|e| e.to_string());
        }
        if timeout.map(|t| started.elapsed() > t).unwrap_or(false) {
            failure = Some("Encoder check timed out.".into());
            let _ = child.kill();
            break child.wait().map_err(|e| e.to_string());
        }
        let percent = total.percent(completed.load(Ordering::Relaxed));
        if percent as i32 != last {
            progress(percent, stage);
            last = percent as i32;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                break Err(e.to_string());
            }
        }
    };
    let _ = reader.join();
    let errors = errors.join().unwrap_or_default();
    if let Some(error) = failure {
        return Err(error);
    }
    check(cancel)?;
    if !status?.success() {
        return Err(format!(
            "FFmpeg could not complete the export. {}",
            String::from_utf8_lossy(&errors[errors.len().saturating_sub(4000)..])
        ));
    }
    match total {
        ProgressTotal::Seconds(value) | ProgressTotal::Frames(value) if value > 0.0 => {
            progress(99.0, stage);
        }
        _ => {}
    }
    Ok(frames.load(Ordering::Relaxed))
}
fn encoder(
    ffmpeg: &Path,
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
) -> Result<String, String> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, String>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(value) = cache.lock().unwrap().get(ffmpeg) {
        return Ok(value.clone());
    }
    for name in ["h264_nvenc", "h264_qsv", "h264_amf"] {
        let args = [
            "-hide_banner",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "nullsrc=s=256x144",
            "-frames:v",
            "1",
            "-c:v",
            name,
            "-f",
            "null",
            "-",
        ]
        .map(String::from);
        if run(
            ffmpeg,
            &args,
            cancel,
            progress,
            ProgressTotal::Seconds(0.0),
            "Checking hardware encoder",
            Some(Duration::from_secs(5)),
        )
        .is_ok()
        {
            cache
                .lock()
                .unwrap()
                .insert(ffmpeg.to_path_buf(), name.into());
            return Ok(name.into());
        }
        check(cancel)?;
    }
    cache
        .lock()
        .unwrap()
        .insert(ffmpeg.to_path_buf(), "libx264".into());
    Ok("libx264".into())
}
fn explicit(options: &Value) -> bool {
    n(&options["scale_percent"]) != 100.0
        || n(&options["output_width"]) > 0.0
        || n(&options["output_height"]) > 0.0
}
fn dimensions(info: &Value, options: &Value, bitrate: f64, even: bool) -> (u32, u32) {
    let w = n(&info["width"]);
    let h = n(&info["height"]);
    let mut scale = n(&options["scale_percent"]) / 100.0;
    for (limit, current) in [
        (n(&options["output_width"]), w),
        (n(&options["output_height"]), h),
        (n(&options["max_height"]), h),
    ] {
        if limit > 0.0 {
            scale = scale.min(limit / current);
        }
    }
    if bitrate > 0.0 && n(&options["max_height"]) > 0.0 && !explicit(options) {
        let fps = if n(&options["fps"]) > 0.0 {
            n(&info["fps"]).min(n(&options["fps"]))
        } else {
            n(&info["fps"])
        };
        scale = scale.min((bitrate / fps.max(1.0) / 0.045 / (w * h)).sqrt());
    }
    let unit = if even && (explicit(options) || n(&options["max_height"]) > 0.0) {
        2
    } else {
        1
    };
    (
        ((w * scale) as u32 / unit * unit).max(unit),
        ((h * scale) as u32 / unit * unit).max(unit),
    )
}
fn plan(info: &Value, target: f64, o: &Value) -> Result<Value, String> {
    let budget = target.min(n(&info["size"]));
    let rate = (budget - 4096.0_f64.min(budget * 0.05)) * 8.0 * 0.92 / n(&info["duration"]);
    let video = s(info, "kind") == "video";
    let has_audio = b(info, "has_audio") && !(video && b(o, "mute_audio"));
    let (mut ar, mut vr) = (0.0, 0.0);
    if s(o, "rate_control") == "target" {
        if video {
            if has_audio {
                ar = (n(&o["audio_bitrate_kbps"]) * 1000.0).min((rate * 0.2).floor());
            }
            vr = (rate - ar).floor().min(100_000_000.0);
            vr = vr.max(12000.0);
            if has_audio {
                ar = ar.max(12000.0);
            }
        } else {
            ar = rate.floor().min(if b(o, "advanced_enabled") {
                n(&o["audio_bitrate_kbps"]) * 1000.0
            } else {
                192000.0
            });
        }
    } else {
        if has_audio {
            ar = n(&o["audio_bitrate_kbps"]) * 1000.0;
        }
        if video && s(o, "rate_control") == "bitrate" {
            vr = n(&o["video_bitrate_kbps"]) * 1000.0;
        }
    }
    if !video {
        if s(o, "audio_format") == "mp3" {
            ar = [
                8000, 16000, 24000, 32000, 40000, 48000, 56000, 64000, 80000, 96000, 112000,
                128000, 160000, 192000, 224000, 256000, 320000,
            ]
            .iter()
            .map(|&v| v as f64)
            .filter(|&v| v <= ar)
            .last()
            .unwrap_or(8000.0);
        }
        ar = ar.max(if s(o, "audio_format") == "ogg" {
            32000.0
        } else {
            6000.0
        });
    }
    let (w, h) = if video {
        dimensions(
            info,
            o,
            if s(o, "rate_control") == "target" {
                vr
            } else {
                0.0
            },
            true,
        )
    } else {
        (0, 0)
    };
    Ok(json!({"video_rate":vr,"audio_rate":ar,"width":w,"height":h,"has_audio":has_audio}))
}
fn add(args: &mut Vec<String>, values: &[&str]) {
    args.extend(values.iter().map(|v| (*v).into()));
}
fn audio_args(
    args: &mut Vec<String>,
    o: &Value,
    info: &Value,
    codec: &str,
    rate: f64,
) -> Result<(), String> {
    let channels = if b(o, "advanced_enabled") {
        if n(&o["audio_channels"]) > 0.0 {
            n(&o["audio_channels"])
        } else {
            n(&info["audio_channels"]).clamp(1.0, 2.0)
        }
    } else {
        2.0
    };
    let sr = if n(&o["audio_sample_rate"]) > 0.0 {
        n(&o["audio_sample_rate"])
    } else if codec == "libmp3lame" && rate < 32000.0 {
        22050.0
    } else {
        48000.0
    };
    if codec == "libopus" && sr != 48000.0 {
        return Err("Opus requires 48,000 Hz from the available sample rates. Choose Automatic or 48,000 Hz.".into());
    }
    let rate = if s(o, "rate_control") == "target" && codec == "libmp3lame" && sr >= 32000.0 {
        rate.max(32000.0)
    } else {
        rate
    };
    if s(o, "compression_mode") == "auto" && codec == "flac" {
        add(
            args,
            &[
                "-ac",
                &(n(&info["audio_channels"]) as u32).to_string(),
                "-ar",
                &(n(&info["audio_sample_rate"]) as u32).to_string(),
            ],
        );
        return Ok(());
    }
    if codec == "libmp3lame"
        && ((sr == 22050.0 && rate > 160000.0) || (sr >= 32000.0 && rate < 32000.0))
    {
        return Err("This MP3 bitrate and sample rate cannot be combined. Choose Automatic sample rate or change the bitrate.".into());
    }
    if codec == "libvorbis" && s(o, "rate_control") == "target" && rate < 48000.0 * channels {
        add(args, &["-q:a", "-1"]);
    } else {
        add(args, &["-b:a", &(rate as u64).to_string()]);
    }
    add(
        args,
        &[
            "-ac",
            &(channels as u32).to_string(),
            "-ar",
            &(sr as u32).to_string(),
        ],
    );
    Ok(())
}
fn preset_args(args: &mut Vec<String>, codec: &str, o: &Value) {
    let index = settings::PRESETS
        .iter()
        .position(|&p| p == s(o, "preset"))
        .unwrap_or(2);
    match codec {
        "libx264" => add(args, &["-preset", s(o, "preset")]),
        "libvpx-vp9" => add(
            args,
            &[
                "-deadline",
                "good",
                "-cpu-used",
                &7usize.saturating_sub(index).to_string(),
                "-row-mt",
                "1",
            ],
        ),
        "h264_nvenc" => add(
            args,
            &["-rc", "vbr", "-preset", &format!("p{}", (2 + index).min(7))],
        ),
        "h264_amf" => add(
            args,
            &[
                "-rc",
                "vbr_peak",
                "-quality",
                if index < 3 {
                    "speed"
                } else if index < 6 {
                    "balanced"
                } else {
                    "quality"
                },
            ],
        ),
        "h264_qsv" => add(
            args,
            &[
                "-preset",
                if index < 3 {
                    "veryfast"
                } else if index < 6 {
                    "medium"
                } else {
                    "veryslow"
                },
            ],
        ),
        _ => {}
    }
}
fn can_copy(source: &Path, target: f64, o: &Value, info: &Value) -> bool {
    let (v, a) = if s(o, "video_format") == "webm" {
        ("vp9", "opus")
    } else {
        ("h264", "aac")
    };
    !b(o, "advanced_enabled")
        && s(info, "kind") == "video"
        && ext(source) == s(o, "video_format")
        && n(&info["size"]) <= target
        && s(info, "codec") == v
        && (!b(info, "has_audio") || s(info, "audio_codec") == a)
        && (n(&o["max_height"]) == 0.0 || n(&info["height"]) <= n(&o["max_height"]))
}
fn encode_media(
    source: &Path,
    output: &Path,
    target: f64,
    options: &Value,
    info: &Value,
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
    sample: Option<(f64, f64)>,
) -> Result<String, String> {
    let preserve = can_copy(source, target, options, info);
    if preserve && sample.is_none() {
        let mut input = File::open(source).map_err(|e| e.to_string())?;
        let mut dest = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output)
            .map_err(|e| e.to_string())?;
        let mut buf = vec![0; 1024 * 1024];
        let mut total = 0;
        loop {
            check(cancel)?;
            let count = input.read(&mut buf).map_err(|e| e.to_string())?;
            if count == 0 {
                break;
            }
            dest.write_all(&buf[..count]).map_err(|e| e.to_string())?;
            total += count;
            progress(
                total as f64 / n(&info["size"]) * 99.0,
                "Already within limit, preserving original quality",
            );
        }
        return Ok("copy".into());
    }
    let mut o = options.clone();
    let original_odd = (!explicit(&o) && n(&o["max_height"]) == 0.0)
        && (n(&info["width"]) as u32 % 2 != 0 || n(&info["height"]) as u32 % 2 != 0);
    if original_odd {
        o["encoder"] = json!("software");
    }
    if preserve {
        o["rate_control"] = json!("quality");
        o["crf"] = json!(0);
        o["encoder"] = json!("software");
    }
    let ffmpeg = tool("ffmpeg")
        .ok_or("FFmpeg was not found. Put ffmpeg and ffprobe in the tools folder.")?;
    let duration = sample.map(|v| v.1).unwrap_or(n(&info["duration"]));
    let video = s(info, "kind") == "video";
    let mut codec = if video {
        if s(&o, "video_format") == "webm" {
            "libvpx-vp9"
        } else {
            "libx264"
        }
    } else {
        match s(&o, "audio_format") {
            "mp3" => "libmp3lame",
            "m4a" | "aac" => "aac",
            "ogg" => "libvorbis",
            "flac" => "flac",
            _ => "libopus",
        }
    }
    .to_string();
    if video && codec == "libx264" && s(&o, "encoder") == "auto" {
        codec = encoder(&ffmpeg, cancel, progress)?;
    }
    let p = plan(info, target, &o)?;
    let mut factor = 1.0;
    let best_output = output.with_extension("best");
    let mut best_size = f64::INFINITY;
    for attempt in 0..6 {
        check(cancel)?;
        let mut args = vec![];
        add(
            &mut args,
            &["-hide_banner", "-loglevel", "error", "-nostdin", "-y"],
        );
        if let Some((start, _)) = sample {
            add(&mut args, &["-ss", &start.to_string()]);
        }
        add(
            &mut args,
            &[
                "-i",
                &source.to_string_lossy(),
                "-map_metadata",
                if b(&o, "strip_metadata") { "-1" } else { "0" },
                "-map_chapters",
                "-1",
                "-sn",
                "-dn",
            ],
        );
        if b(&o, "strip_metadata") {
            add(&mut args, &["-map_metadata:s", "-1"]);
        }
        if video {
            let ar = (n(&p["audio_rate"]) * factor).floor().max(12000.0);
            let vr = (n(&p["video_rate"]) * factor).floor().max(12000.0);
            let (w, h) = dimensions(
                info,
                &o,
                if s(&o, "rate_control") == "target" {
                    vr
                } else {
                    0.0
                },
                true,
            );
            let mut filter = format!("scale={w}:{h},setsar=1");
            if n(&o["fps"]) > 0.0 {
                filter += &format!(",fps={}", n(&o["fps"]).min(n(&info["fps"])));
            }
            add(
                &mut args,
                &[
                    "-map",
                    &format!("0:{}", info["video_index"]),
                    "-vf",
                    &filter,
                    "-pix_fmt",
                    if preserve || s(&o, "compression_mode") == "auto" {
                        s(info, "pixel_format")
                    } else if original_odd {
                        "yuv444p"
                    } else {
                        "yuv420p"
                    },
                    "-c:v",
                    &codec,
                ],
            );
            if s(&o, "rate_control") == "quality" {
                add(&mut args, &["-crf", &n(&o["crf"]).to_string()]);
                if codec == "libvpx-vp9" {
                    add(&mut args, &["-b:v", "0"]);
                    if preserve {
                        add(&mut args, &["-lossless", "1"]);
                    }
                }
            } else {
                add(
                    &mut args,
                    &[
                        "-b:v",
                        &vr.to_string(),
                        "-maxrate",
                        &vr.to_string(),
                        "-bufsize",
                        &(vr * 2.0).to_string(),
                    ],
                );
            }
            preset_args(&mut args, &codec, &o);
            if s(&o, "compression_mode") == "auto" && s(&o, "video_format") == "mkv" {
                add(
                    &mut args,
                    &[
                        "-map", "0:a?", "-c:a", "copy", "-map", "0:s?", "-c:s", "copy", "-map",
                        "0:t?", "-c:t", "copy",
                    ],
                );
            } else if b(&p, "has_audio") {
                let ac = if s(&o, "video_format") == "webm" {
                    "libopus"
                } else {
                    "aac"
                };
                add(
                    &mut args,
                    &[
                        "-map",
                        &format!("0:{}", info["audio_index"]),
                        "-c:a",
                        if preserve { "copy" } else { ac },
                    ],
                );
                if !preserve {
                    audio_args(&mut args, &o, info, ac, ar)?;
                }
            } else {
                add(&mut args, &["-an"]);
            }
            if ["mp4", "mov"].contains(&s(&o, "video_format")) {
                add(&mut args, &["-movflags", "+faststart"]);
            }
        } else {
            let ar = n(&plan(info, target * factor, &o)?["audio_rate"]);
            add(
                &mut args,
                &[
                    "-map",
                    &format!("0:{}", info["audio_index"]),
                    "-vn",
                    "-c:a",
                    &codec,
                ],
            );
            audio_args(&mut args, &o, info, &codec, ar)?;
        }
        add(
            &mut args,
            &[
                "-t",
                &duration.to_string(),
                "-progress",
                "pipe:1",
                "-nostats",
                &output.to_string_lossy(),
            ],
        );
        if let Err(error) = run(
            &ffmpeg,
            &args,
            cancel,
            progress,
            ProgressTotal::media(info, &o, duration, sample.is_some()),
            if attempt == 0 {
                "Compressing"
            } else {
                "Fitting size limit"
            },
            None,
        ) {
            check(cancel)?;
            if ["h264_nvenc", "h264_qsv", "h264_amf"].contains(&codec.as_str()) {
                codec = "libx264".into();
                progress(0.0, "Retrying with software encoder");
                continue;
            }
            return Err(error);
        }
        let size = fs::metadata(output).map_err(|e| e.to_string())?.len() as f64;
        if size <= 0.0 {
            return Err("The encoder produced an empty file.".into());
        }
        if sample.is_some() || s(&o, "rate_control") != "target" || size <= target {
            return Ok(codec);
        }
        if size < best_size {
            fs::copy(output, &best_output).map_err(|e| e.to_string())?;
            best_size = size;
        }
        factor *= 0.8_f64.min(target / size * 0.85);
    }
    fs::copy(&best_output, output).map_err(|e| e.to_string())?;
    Ok(codec)
}
fn image_metadata(path: &Path) -> Result<(Vec<u8>, Vec<u8>), String> {
    let data = fs::read(path).map_err(|e| e.to_string())?;
    let (mut exif, mut icc) = (Vec::new(), Vec::new());
    if data.starts_with(&[255, 216]) {
        let mut offset = 2;
        let mut icc_parts = Vec::new();
        while offset + 4 <= data.len() && data[offset] == 255 {
            let marker = data[offset + 1];
            if marker == 0xda || marker == 0xd9 {
                break;
            }
            let len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
            if len < 2 || offset + 2 + len > data.len() {
                break;
            }
            let block = &data[offset + 4..offset + 2 + len];
            if marker == 0xe1 && block.starts_with(b"Exif\0\0") {
                exif = block[6..].to_vec();
            }
            if marker == 0xe2 && block.starts_with(b"ICC_PROFILE\0") && block.len() >= 14 {
                icc_parts.push((block[12], block[14..].to_vec()));
            }
            offset += 2 + len;
        }
        icc_parts.sort_by_key(|v| v.0);
        for (_, part) in icc_parts {
            icc.extend(part);
        }
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        let mut offset = 12;
        while offset + 8 <= data.len() {
            let len = u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
            if offset + 8 + len > data.len() {
                break;
            }
            let block = &data[offset + 8..offset + 8 + len];
            match &data[offset..offset + 4] {
                b"EXIF" => exif = block.strip_prefix(b"Exif\0\0").unwrap_or(block).to_vec(),
                b"ICCP" => icc = block.to_vec(),
                _ => {}
            }
            offset += 8 + len + (len % 2);
        }
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        let mut offset = 8;
        while offset + 12 <= data.len() {
            let len = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
            if len > data.len() - offset - 12 {
                break;
            }
            let block = &data[offset + 8..offset + 8 + len];
            match &data[offset + 4..offset + 8] {
                b"eXIf" => exif = block.to_vec(),
                b"iCCP" => {
                    if let Some(zero) = block.iter().position(|&b| b == 0) {
                        if block.get(zero + 1) == Some(&0) {
                            let mut decoder = flate2::read::ZlibDecoder::new(&block[zero + 2..]);
                            decoder
                                .by_ref()
                                .take(16 * 1024 * 1024)
                                .read_to_end(&mut icc)
                                .map_err(|e| e.to_string())?;
                        }
                    }
                }
                _ => {}
            }
            offset += 12 + len;
        }
    } else if data.starts_with(b"II\x2a\0") || data.starts_with(b"MM\0\x2a") {
        exif = data;
    }
    Ok((exif, icc))
}
fn exif_orientation(exif: &mut [u8], normalize: bool) -> u16 {
    if exif.len() >= 8 && (exif.starts_with(b"II") || exif.starts_with(b"MM")) {
        let little = exif.starts_with(b"II");
        let get16 = |v: &[u8]| {
            if little {
                u16::from_le_bytes(v.try_into().unwrap())
            } else {
                u16::from_be_bytes(v.try_into().unwrap())
            }
        };
        let get32 = |v: &[u8]| {
            if little {
                u32::from_le_bytes(v.try_into().unwrap())
            } else {
                u32::from_be_bytes(v.try_into().unwrap())
            }
        };
        let start = get32(&exif[4..8]) as usize;
        if start + 2 <= exif.len() {
            let count = get16(&exif[start..start + 2]) as usize;
            for i in 0..count {
                let p = start + 2 + i * 12;
                if p + 12 > exif.len() {
                    break;
                }
                if get16(&exif[p..p + 2]) == 274 {
                    let orientation = get16(&exif[p + 8..p + 10]);
                    let one = if little {
                        1u16.to_le_bytes()
                    } else {
                        1u16.to_be_bytes()
                    };
                    if normalize {
                        exif[p + 8..p + 10].copy_from_slice(&one);
                    }
                    return if (1..=8).contains(&orientation) {
                        orientation
                    } else {
                        1
                    };
                }
            }
        }
    }
    1
}
fn write_image_metadata(
    output: &Path,
    exif: &[u8],
    icc: &[u8],
    width: u32,
    height: u32,
) -> Result<(), String> {
    if exif.is_empty() && icc.is_empty() {
        return Ok(());
    }
    let data = fs::read(output).map_err(|e| e.to_string())?;
    let mut result = Vec::new();
    if data.starts_with(&[255, 216]) {
        result.extend([255, 216]);
        let mut segment = |marker: u8, bytes: &[u8]| -> Result<(), String> {
            if bytes.len() > 65533 {
                return Err("Image metadata is too large for JPEG.".into());
            }
            result.extend([255, marker]);
            result.extend(((bytes.len() + 2) as u16).to_be_bytes());
            result.extend(bytes);
            Ok(())
        };
        if !exif.is_empty() {
            let mut bytes = b"Exif\0\0".to_vec();
            bytes.extend(exif);
            segment(0xe1, &bytes)?;
        }
        let count = icc.len().div_ceil(65519);
        if count > 255 {
            return Err("Image color profile is too large for JPEG.".into());
        }
        for (index, part) in icc.chunks(65519).enumerate() {
            let mut bytes = b"ICC_PROFILE\0".to_vec();
            bytes.extend([(index + 1) as u8, count as u8]);
            bytes.extend(part);
            segment(0xe2, &bytes)?;
        }
        result.extend(&data[2..]);
    } else if data.starts_with(b"RIFF") && data.get(8..12) == Some(b"WEBP") {
        let mut chunks: Vec<u8> = Vec::new();
        let mut offset = 12;
        let mut flags = 0u8;
        while offset + 8 <= data.len() {
            let size =
                u32::from_le_bytes(data[offset + 4..offset + 8].try_into().unwrap()) as usize;
            let end = offset + 8 + size + size % 2;
            if end > data.len() {
                return Err("Invalid WebP output.".into());
            }
            let tag = &data[offset..offset + 4];
            if tag == b"VP8X" {
                flags = data[offset + 8];
            } else if tag != b"EXIF" && tag != b"ICCP" {
                if tag == b"ALPH" {
                    flags |= 0x10;
                }
                if tag == b"VP8L" && size >= 5 && data[offset + 12] & 0x10 != 0 {
                    flags |= 0x10;
                }
                chunks.extend(&data[offset..end]);
            }
            offset = end;
        }
        flags &= !0x28;
        if !icc.is_empty() {
            flags |= 0x20;
        }
        if !exif.is_empty() {
            flags |= 0x08;
        }
        result.extend(b"RIFF\0\0\0\0WEBP");
        result.extend(b"VP8X\x0a\0\0\0");
        result.extend([flags, 0, 0, 0]);
        result.extend(&(width - 1).to_le_bytes()[..3]);
        result.extend(&(height - 1).to_le_bytes()[..3]);
        let append = |target: &mut Vec<u8>, tag: &[u8], bytes: &[u8]| {
            target.extend(tag);
            target.extend((bytes.len() as u32).to_le_bytes());
            target.extend(bytes);
            if bytes.len() % 2 == 1 {
                target.push(0);
            }
        };
        if !icc.is_empty() {
            append(&mut result, b"ICCP", icc);
        }
        result.extend(chunks);
        if !exif.is_empty() {
            append(&mut result, b"EXIF", exif);
        }
        let len = (result.len() - 8) as u32;
        result[4..8].copy_from_slice(&len.to_le_bytes());
    } else {
        return Err("Unsupported image metadata container.".into());
    }
    fs::write(output, result).map_err(|e| e.to_string())
}
fn encode_image(
    source: &Path,
    output: &Path,
    target: f64,
    o: &Value,
    info: &Value,
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
) -> Result<String, String> {
    let ffmpeg = tool("ffmpeg").ok_or("FFmpeg was not found.")?;
    let (mut exif, icc) = if b(o, "strip_metadata") {
        (Vec::new(), Vec::new())
    } else {
        image_metadata(source)?
    };
    exif_orientation(&mut exif, true);
    let orientation = match n(&info["orientation"]) as u16 {
        2 => "hflip,",
        3 => "hflip,vflip,",
        4 => "vflip,",
        5 => "transpose=clock,hflip,",
        6 => "transpose=clock,",
        7 => "transpose=clock,vflip,",
        8 => "transpose=cclock,",
        _ => "",
    };
    let (mut width, mut height) = dimensions(info, o, 0.0, false);
    let jpeg = s(o, "image_format") == "jpeg";
    let lossless = b(o, "image_lossless") && !jpeg;
    let max = if b(o, "advanced_enabled") {
        n(&o["image_quality"]) as i32
    } else {
        95
    };
    let best_output = output.with_extension("best");
    let mut smallest = u64::MAX;
    let mut encode = |quality: i32, w: u32, h: u32| -> Result<u64, String> {
        check(cancel)?;
        let mut args = vec![];
        add(
            &mut args,
            &[
                "-hide_banner",
                "-loglevel",
                "error",
                "-nostdin",
                "-y",
                "-noautorotate",
                "-i",
                &source.to_string_lossy(),
                "-map_metadata",
                if b(o, "strip_metadata") { "-1" } else { "0" },
            ],
        );
        if jpeg {
            add(&mut args,&["-filter_complex",&format!("[0:v]{orientation}scale={w}:{h}:flags=lanczos,format=rgba[fg];color=white:s={w}x{h},format=rgba[bg];[bg][fg]overlay=shortest=1,format=yuvj444p[out]"),"-map","[out]","-c:v","mjpeg","-q:v",&(2+(100-quality)*29/99).to_string()]);
        } else {
            add(
                &mut args,
                &[
                    "-vf",
                    &format!("{orientation}scale={w}:{h}:flags=lanczos,format=rgba"),
                    "-c:v",
                    "libwebp",
                    "-quality",
                    &quality.to_string(),
                    "-lossless",
                    if lossless { "1" } else { "0" },
                    "-compression_level",
                    "4",
                ],
            );
        }
        add(
            &mut args,
            &["-frames:v", "1", "-update", "1", &output.to_string_lossy()],
        );
        run(
            &ffmpeg,
            &args,
            cancel,
            progress,
            ProgressTotal::Seconds(0.0),
            "Compressing image",
            None,
        )?;
        write_image_metadata(output, &exif, &icc, w, h)?;
        let size = fs::metadata(output).map_err(|e| e.to_string())?.len();
        if size > 0 && size < smallest {
            fs::copy(output, &best_output).map_err(|e| e.to_string())?;
            smallest = size;
        }
        Ok(size)
    };
    if s(o, "rate_control") != "target" {
        encode(max, width, height)?;
        return Ok(if jpeg { "mjpeg" } else { "libwebp" }.into());
    }
    for attempt in 0..40 {
        let (mut low, mut high) = if lossless { (max, max) } else { (1, max) };
        let mut best = None;
        while low <= high {
            let q = (low + high) / 2;
            progress((attempt as f64 * 5.0).min(95.0), "Compressing image");
            let size = encode(q, width, height)?;
            if size > 0 && (size as f64) <= target {
                best = Some(q);
                low = q + 1;
            } else {
                high = q - 1;
            }
        }
        if let Some(q) = best {
            encode(q, width, height)?;
            return Ok(if jpeg { "mjpeg" } else { "libwebp" }.into());
        }
        if (width == 1 && height == 1) || explicit(o) || n(&o["max_height"]) == 0.0 {
            break;
        }
        width = ((width as f64 * 0.75) as u32).max(1);
        height = ((height as f64 * 0.75) as u32).max(1);
    }
    fs::copy(&best_output, output).map_err(|e| e.to_string())?;
    Ok(if jpeg { "mjpeg" } else { "libwebp" }.into())
}
fn decoded_hash(
    path: &Path,
    kind: &str,
    orientation: u16,
    cancel: &AtomicBool,
) -> Result<String, String> {
    let ffmpeg = tool("ffmpeg").ok_or("FFmpeg was not found.")?;
    let mut args = vec![
        "-hide_banner".into(),
        "-v".into(),
        "error".into(),
        "-noautorotate".into(),
        "-i".into(),
        path.to_string_lossy().into_owned(),
    ];
    if kind == "audio" {
        add(&mut args, &["-map", "0:a:0", "-c:a", "pcm_s32le"]);
    } else {
        let transform = match orientation {
            2 => "hflip,",
            3 => "hflip,vflip,",
            4 => "vflip,",
            5 => "transpose=clock,hflip,",
            6 => "transpose=clock,",
            7 => "transpose=clock,vflip,",
            8 => "transpose=cclock,",
            _ => "",
        };
        add(
            &mut args,
            &[
                "-map",
                "0:v:0",
                "-vf",
                &format!("{transform}format=rgba"),
                "-frames:v",
                "1",
            ],
        );
    }
    add(&mut args, &["-f", "hash", "-hash", "SHA256", "-"]);
    let output = capture(&ffmpeg, &args, cancel, Duration::from_secs(7200))?;
    let digest = String::from_utf8_lossy(&output).trim().to_string();
    if !digest.starts_with("SHA256=") {
        return Err("Decoded content could not be compared.".into());
    }
    Ok(digest)
}
fn parse_frame_count(data: &str) -> Result<u64, String> {
    let mut counts = data
        .lines()
        .filter_map(|line| line.trim().parse::<u64>().ok());
    let first = counts
        .next()
        .filter(|count| *count > 0)
        .ok_or("Video frames could not be counted.")?;
    if counts.all(|count| count == first) {
        Ok(first)
    } else {
        Err("Video frame counts disagree between streams.".into())
    }
}
fn counted_frames(path: &Path, cancel: &AtomicBool) -> Result<u64, String> {
    let ffprobe = tool("ffprobe").ok_or("FFprobe was not found.")?;
    let args = [
        "-v",
        "error",
        "-select_streams",
        "v:0",
        "-count_frames",
        "-show_entries",
        "stream=nb_read_frames",
        "-of",
        "default=noprint_wrappers=1:nokey=1",
        &path.to_string_lossy(),
    ]
    .map(String::from);
    parse_frame_count(&String::from_utf8_lossy(&capture(
        &ffprobe,
        &args,
        cancel,
        Duration::from_secs(7200),
    )?))
}
fn source_frame_count(info: &Value, source: &Path, cancel: &AtomicBool) -> Result<u64, String> {
    let frames = n(&info["frame_count"]);
    if frames > 0.0 && frames.fract() == 0.0 {
        Ok(frames as u64)
    } else {
        counted_frames(source, cancel)
    }
}
fn visual_score(
    source: &Path,
    output: &Path,
    info: &Value,
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
    sample: Option<(f64, f64)>,
    source_frames: Option<u64>,
) -> Result<(f64, f64), String> {
    let ffmpeg = tool("ffmpeg").ok_or("FFmpeg was not found.")?;
    let stats = output.with_extension("ssim-stats");
    let (w, h) = (n(&info["width"]) as u32, n(&info["height"]) as u32);
    let fps = n(&info["fps"]).max(1.0);
    let filter = format!("[0:v]settb=AVTB,setpts=N/({fps}*TB),scale={w}:{h},setsar=1,format=yuv444p16le[ref];[1:v]settb=AVTB,setpts=N/({fps}*TB),format=yuv444p16le[encoded];[encoded][ref]ssim=eof_action=pass:repeatlast=0:stats_file={}[comparison]", stats.to_string_lossy().replace('\\', "/").replace(':', "\\\\\\:"));
    let mut args = vec![];
    add(&mut args, &["-hide_banner", "-v", "error", "-nostdin"]);
    if let Some((start, duration)) = sample {
        add(
            &mut args,
            &["-ss", &start.to_string(), "-t", &duration.to_string()],
        );
    }
    add(
        &mut args,
        &[
            "-i",
            &source.to_string_lossy(),
            "-i",
            &output.to_string_lossy(),
            "-filter_complex",
            &filter,
            "-map",
            "[comparison]",
            "-an",
            "-progress",
            "pipe:1",
            "-f",
            "null",
            "-",
        ],
    );
    let produced = run(
        &ffmpeg,
        &args,
        cancel,
        progress,
        ProgressTotal::media(
            info,
            &json!({}),
            sample.map_or(n(&info["duration"]), |(_, duration)| duration),
            sample.is_some(),
        ),
        "Comparing visual quality",
        None,
    )?;
    let data = fs::read_to_string(&stats).map_err(|e| e.to_string())?;
    let scores: Vec<f64> = data
        .lines()
        .filter_map(|line| {
            line.split_whitespace().find_map(|field| {
                field
                    .strip_prefix("All:")
                    .and_then(|score| score.parse().ok())
            })
        })
        .collect();
    let expected = if sample.is_some() {
        None
    } else if let Some(frames) = source_frames {
        Some(frames)
    } else {
        Some(source_frame_count(info, source, cancel)?)
    };
    if produced == 0
        || scores.len() as u64 != produced
        || expected.is_some_and(|frames| frames != produced)
    {
        return Err(format!("Video frame counts changed during auto compression: source {}, output {produced}, compared {}.", expected.map_or("sample".to_owned(), |frames| frames.to_string()), scores.len()));
    }
    let mean = scores.iter().sum::<f64>() / scores.len() as f64;
    let worst = scores.iter().copied().fold(1.0, f64::min);
    Ok((mean, worst))
}
fn auto_sample_windows(duration: f64, pixels_per_second: f64) -> Vec<(f64, f64)> {
    // Sampling lost to full search at 59 million pixels and won at 221 million on the benchmark fixture.
    if duration < 20.0 || duration * pixels_per_second < 200_000_000.0 {
        return vec![];
    }
    let length = (duration / 8.0).min(0.75);
    let last = duration - length;
    (0..5)
        .map(|index| (last * (index as f64 + 0.5) / 5.0, length))
        .collect()
}
fn sampled_quality_passes(scores: &[(f64, f64)]) -> bool {
    !scores.is_empty()
        && scores.iter().map(|score| score.0).sum::<f64>() / scores.len() as f64 >= 0.9955
        && scores.iter().all(|score| score.1 >= 0.9865)
}
fn sampled_auto_candidate(
    source: &Path,
    output: &Path,
    info: &Value,
    options: &Value,
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
) -> Result<Option<String>, String> {
    let windows = auto_sample_windows(
        n(&info["duration"]),
        n(&info["width"]) * n(&info["height"]) * n(&info["fps"]),
    );
    if windows.is_empty() {
        return Ok(None);
    }
    let sample_output = output.with_extension("auto-sample.mkv");
    let original_size = n(&info["size"]) as u64;
    let mut low = 8;
    let mut high = 35;
    let mut chosen = None;
    let mut extended = false;
    for attempt in 0..9 {
        if low > high {
            if chosen == Some(35) && !extended {
                low = 36;
                high = 51;
                extended = true;
            } else {
                break;
            }
        }
        check(cancel)?;
        let crf = (low + high) / 2;
        let mut trial = options.clone();
        trial["crf"] = json!(crf);
        trial["preset"] = json!("slow");
        let mut scores = Vec::with_capacity(windows.len());
        for (index, window) in windows.iter().copied().enumerate() {
            let encode_progress = |percent: f64, _: &str| {
                progress(
                    ((attempt as f64 + (index as f64 + percent / 200.0) / windows.len() as f64)
                        / 9.0
                        * 30.0)
                        .min(30.0),
                    "Auto quality: sampling candidate",
                )
            };
            encode_media(
                source,
                &sample_output,
                original_size as f64,
                &trial,
                info,
                cancel,
                &encode_progress,
                Some(window),
            )?;
            let sample_info = probe_cancel(&sample_output, cancel)?;
            if sample_info["width"] != info["width"] || sample_info["height"] != info["height"] {
                return Ok(None);
            }
            let compare_progress = |percent: f64, _: &str| {
                progress(
                    ((attempt as f64
                        + (index as f64 + 0.5 + percent / 200.0) / windows.len() as f64)
                        / 9.0
                        * 30.0)
                        .min(30.0),
                    "Auto quality: comparing sample",
                )
            };
            let score = match visual_score(
                source,
                &sample_output,
                info,
                cancel,
                &compare_progress,
                Some(window),
                None,
            ) {
                Ok(score) => score,
                Err(error) if error.starts_with("Video frame counts changed") => return Ok(None),
                Err(error) => return Err(error),
            };
            scores.push(score);
        }
        if sampled_quality_passes(&scores) {
            chosen = Some(crf);
            low = crf + 1;
        } else {
            high = crf - 1;
        }
    }
    let Some(crf) = chosen else {
        return Ok(None);
    };
    let source_frames = source_frame_count(info, source, cancel)?;
    let candidates = if crf > 0 {
        vec![crf, (crf - 4).max(0)]
    } else {
        vec![crf]
    };
    for (attempt, crf) in candidates.into_iter().enumerate() {
        check(cancel)?;
        let mut trial = options.clone();
        trial["crf"] = json!(crf);
        trial["preset"] = json!("slow");
        let (base, encode_span, compare_span) = if attempt == 0 {
            (30.0, 40.0, 15.0)
        } else {
            (85.0, 9.0, 4.0)
        };
        let encode_progress = |percent: f64, _: &str| {
            progress(
                base + percent / 100.0 * encode_span,
                &format!(
                    "Auto quality: encoding full file (pass {}, {percent:.0}%)",
                    attempt + 1
                ),
            )
        };
        let codec = encode_media(
            source,
            output,
            original_size as f64,
            &trial,
            info,
            cancel,
            &encode_progress,
            None,
        )?;
        let encoded = probe_cancel(output, cancel)?;
        if encoded["width"] != info["width"]
            || encoded["height"] != info["height"]
            || (n(&encoded["duration"]) - n(&info["duration"])).abs() > 0.1
        {
            continue;
        }
        let compare_progress = |percent: f64, _: &str| {
            progress(
                base + encode_span + percent / 100.0 * compare_span,
                &format!(
                    "Auto quality: verifying full file (pass {}, {percent:.0}%)",
                    attempt + 1
                ),
            )
        };
        let (mean, worst) = match visual_score(
            source,
            output,
            info,
            cancel,
            &compare_progress,
            None,
            Some(source_frames),
        ) {
            Ok(score) => score,
            Err(error) if error.starts_with("Video frame counts changed") => continue,
            Err(error) => return Err(error),
        };
        if mean >= 0.995
            && worst >= 0.985
            && fs::metadata(output).map_err(|e| e.to_string())?.len() < original_size
        {
            progress(98.0, "Auto quality: verified full file");
            return Ok(Some(codec));
        }
    }
    Ok(None)
}
fn auto_candidate(
    source: &Path,
    output: &Path,
    info: &Value,
    options: &Value,
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
) -> Result<Option<String>, String> {
    let original_size = n(&info["size"]) as u64;
    if s(info, "kind") == "video" {
        let sampled = !auto_sample_windows(
            n(&info["duration"]),
            n(&info["width"]) * n(&info["height"]) * n(&info["fps"]),
        )
        .is_empty();
        let fallback_base = if sampled { 70.0 } else { 0.0 };
        let fallback_span = if sampled { 24.0 } else { 94.0 };
        let reported = Mutex::new(0.0_f64);
        let report = |percent: f64, stage: &str| {
            let mut current = reported.lock().unwrap();
            *current = current.max(percent);
            progress(*current, stage);
        };
        if sampled {
            return sampled_auto_candidate(source, output, info, options, cancel, &report);
        }
        let mut low = 8;
        let mut high = 51;
        let mut best = None;
        let best_path = output.with_extension("auto-best");
        let source_frames = source_frame_count(info, source, cancel)?;
        for attempt in 0..6 {
            if low > high {
                break;
            }
            check(cancel)?;
            let crf = (low + high) / 2;
            let mut trial = options.clone();
            trial["crf"] = json!(crf);
            trial["preset"] = json!("slow");
            let encode_progress = |percent: f64, _: &str| {
                report(
                    (fallback_base + (attempt as f64 + percent / 200.0) / 6.0 * fallback_span)
                        .min(94.0),
                    "Auto quality: encoding candidate",
                )
            };
            let codec = encode_media(
                source,
                output,
                original_size as f64,
                &trial,
                info,
                cancel,
                &encode_progress,
                None,
            )?;
            let encoded = probe_cancel(output, cancel)?;
            if encoded["width"] != info["width"]
                || encoded["height"] != info["height"]
                || (n(&encoded["duration"]) - n(&info["duration"])).abs() > 0.1
            {
                return Ok(None);
            }
            let compare_progress = |percent: f64, _: &str| {
                report(
                    (fallback_base
                        + (attempt as f64 + 0.5 + percent / 200.0) / 6.0 * fallback_span)
                        .min(94.0),
                    "Auto quality: comparing frames",
                )
            };
            let (mean, worst) = match visual_score(
                source,
                output,
                info,
                cancel,
                &compare_progress,
                None,
                Some(source_frames),
            ) {
                Ok(score) => score,
                Err(error) if error.starts_with("Video frame counts changed") => return Ok(None),
                Err(error) => return Err(error),
            };
            if mean >= 0.995 && worst >= 0.985 {
                let size = fs::metadata(output).map_err(|e| e.to_string())?.len();
                if size < original_size
                    && best
                        .as_ref()
                        .map_or(true, |(previous, _): &(u64, String)| size < *previous)
                {
                    fs::copy(output, &best_path).map_err(|e| e.to_string())?;
                    best = Some((size, codec));
                }
                low = crf + 1;
            } else {
                high = crf - 1;
            }
        }
        if let Some((_, codec)) = best {
            fs::copy(&best_path, output).map_err(|e| e.to_string())?;
            return Ok(Some(codec));
        }
        return Ok(None);
    }
    let codec = if s(info, "kind") == "image" {
        encode_image(
            source,
            output,
            original_size as f64,
            options,
            info,
            cancel,
            progress,
        )?
    } else {
        encode_media(
            source,
            output,
            original_size as f64,
            options,
            info,
            cancel,
            progress,
            None,
        )?
    };
    if fs::metadata(output).map_err(|e| e.to_string())?.len() >= original_size {
        return Ok(None);
    }
    let lossless = s(info, "kind") == "audio" && s(options, "audio_format") == "flac"
        || s(info, "kind") == "image" && s(options, "image_format") == "webp";
    if !lossless {
        return Ok(Some(codec));
    }
    let original = decoded_hash(
        source,
        s(info, "kind"),
        n(&info["orientation"]) as u16,
        cancel,
    )?;
    let encoded = decoded_hash(output, s(info, "kind"), 1, cancel)?;
    Ok((original == encoded).then_some(codec))
}
pub fn compress(
    source: &Path,
    destination: &Path,
    options: &Value,
    cancel: &AtomicBool,
    progress: impl Fn(f64, &str),
) -> Result<Value, String> {
    export(source, destination, options, cancel, &progress, None)
}
pub fn preview(
    source: &Path,
    destination: &Path,
    options: &Value,
    cancel: &AtomicBool,
    progress: impl Fn(f64, &str),
    start: f64,
    duration: f64,
) -> Result<Value, String> {
    if options["compression_mode"] == "auto" {
        return Err(
            "Auto quality compares the full file. Switch to Size limit for a short preview.".into(),
        );
    }
    if !start.is_finite()
        || !duration.is_finite()
        || start < 0.0
        || !(0.1..=15.0).contains(&duration)
    {
        return Err(
            "Preview duration must be between 0.1 and 15 seconds, with a nonnegative start time."
                .into(),
        );
    }
    export(
        source,
        destination,
        options,
        cancel,
        &progress,
        Some((start, duration)),
    )
}
fn export(
    source: &Path,
    destination: &Path,
    options: &Value,
    cancel: &AtomicBool,
    progress: &impl Fn(f64, &str),
    mut sample: Option<(f64, f64)>,
) -> Result<Value, String> {
    check(cancel)?;
    let mut raw = options.clone();
    if let Some(obj) = raw.as_object_mut() {
        obj.remove("output_dir");
    }
    let o = settings::effective(&raw)?;
    let target = (n(&o["target_mb"]) * 1_000_000.0).floor();
    let source = source.canonicalize().map_err(|e| e.to_string())?;
    if fs::symlink_metadata(destination).is_ok() {
        return Err("The destination already exists. Choose another filename.".into());
    }
    let parent = destination
        .parent()
        .ok_or("The destination folder does not exist.")?;
    if !parent.is_dir() {
        return Err("The destination folder does not exist.".into());
    }
    progress(0.0, "Reading media");
    let info = probe_cancel(&source, cancel)?;
    if let Some((start, duration)) = sample {
        if s(&info, "kind") != "image" {
            if start >= n(&info["duration"]) {
                return Err("Preview start must be before the end of the file.".into());
            }
            sample = Some((start, duration.min(n(&info["duration"]) - start)));
        }
    }
    let extension = s(&o, &format!("{}_format", s(&info, "kind")));
    if ext(destination) != extension && !(extension == "jpeg" && ext(destination) == "jpg") {
        return Err("The destination extension does not match the selected format.".into());
    }
    let temp = tempfile::Builder::new()
        .prefix(".blubberbound-")
        .tempdir_in(parent)
        .map_err(|e| e.to_string())?;
    let output = temp.path().join(format!("output.{extension}"));
    let selected = if s(&o, "compression_mode") == "auto" {
        auto_candidate(&source, &output, &info, &o, cancel, progress)?
    } else if s(&info, "kind") == "image" {
        Some(encode_image(
            &source, &output, target, &o, &info, cancel, progress,
        )?)
    } else {
        Some(encode_media(
            &source, &output, target, &o, &info, cancel, progress, sample,
        )?)
    };
    let Some(codec) = selected else {
        check(cancel)?;
        progress(
            100.0,
            "Original is smallest without detectable quality loss",
        );
        return Ok(
            json!({"path":source,"size":info["size"],"original_size":info["size"],"kind":info["kind"],"encoder":"original","width":info["width"],"height":info["height"],"duration_seconds":info["duration"],"fps":info["fps"],"preserved_original":true,"warning":"Original kept: no smaller output passed the quality checks."}),
        );
    };
    check(cancel)?;
    let size = fs::metadata(&output).map_err(|e| e.to_string())?.len();
    if size == 0 {
        return Err("The encoder produced an empty file.".into());
    }
    progress(99.0, "Checking output");
    let output_info = probe_cancel(&output, cancel)?;
    progress(99.0, "Saving output");
    check(cancel)?;
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = output.as_os_str().encode_wide().chain(Some(0)).collect();
        let target: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                target.as_ptr(),
                0,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
    }
    #[cfg(not(windows))]
    fs::hard_link(&output, destination).map_err(|e| e.to_string())?;
    let mut result = json!({"path":destination,"size":size,"original_size":info["size"],"kind":info["kind"],"encoder":codec,"width":output_info["width"],"height":output_info["height"],"duration_seconds":output_info["duration"],"fps":output_info["fps"]});
    if s(&o, "rate_control") == "target"
        && (sample.is_none() || s(&info, "kind") == "image")
        && size as f64 > target
    {
        result["warning"] = json!(format!("Saved the smallest output produced with these settings ({:.3} MB). It exceeds your {:.3} MB size limit.", size as f64 / 1_000_000.0, target / 1_000_000.0));
    }
    if let Some((start, duration)) = sample {
        let image = s(&info, "kind") == "image";
        result["start_seconds"] = json!(if image { 0.0 } else { start });
        result["duration_seconds"] = json!(if image { 0.0 } else { duration });
        result["source_duration_seconds"] = info["duration"].clone();
        if !image {
            if can_copy(&source, target, &o, &info) {
                result["preserves_original"] = json!(true);
                result["quality_note"] =
                    json!("The full export will keep the original file quality.");
            } else {
                let p = plan(&info, target, &o)?;
                for key in ["video_rate", "audio_rate", "has_audio"] {
                    result[key] = p[key].clone();
                }
            }
        }
    }
    progress(100.0, "Done");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn video_progress_uses_frames_instead_of_buffered_timestamps() {
        let total = ProgressTotal::Frames(4.0);
        assert_eq!(total.count("out_time_us=500000"), None);
        assert_eq!(total.percent(total.count("frame=4").unwrap()), 99.0);
        assert_eq!(total.percent(total.count("frame=3").unwrap()), 74.25);
        assert_eq!(total.count("frame=N/A"), None);
        let audio = ProgressTotal::Seconds(4.0);
        assert_eq!(
            audio.percent(audio.count("out_time_us=2000000").unwrap()),
            49.5
        );
        assert_eq!(audio.count("out_time_us=-100"), None);
        assert_eq!(audio.count("out_time_us=N/A"), None);
        assert_eq!(total.percent(10), 99.0);
        assert_eq!(ProgressTotal::Seconds(0.0).percent(1), 0.0);
    }
    #[test]
    fn progress_totals_follow_frame_count_frame_rate_and_preview_length() {
        let info = json!({"kind":"video","fps":30,"frame_count":240,"video_duration":8});
        let full = ProgressTotal::media(&info, &json!({}), 10.0, false);
        assert_eq!(full.percent(120), 49.5);
        let capped = ProgressTotal::media(&info, &json!({"fps":15}), 10.0, false);
        assert_eq!(capped.percent(60), 49.5);
        let sample = ProgressTotal::media(&info, &json!({"fps":15}), 2.0, true);
        assert_eq!(sample.percent(15), 49.5);
        let variable = json!({"kind":"video","fps":30,"frame_count":150});
        assert_eq!(
            ProgressTotal::media(&variable, &json!({}), 10.0, false).percent(75),
            49.5
        );
        let missing_count = json!({"kind":"video","fps":30});
        assert_eq!(
            ProgressTotal::media(&missing_count, &json!({}), 10.0, false).percent(150),
            49.5
        );
        let audio = json!({"kind":"audio"});
        assert_eq!(
            ProgressTotal::media(&audio, &json!({}), 10.0, false).percent(5_000_000),
            49.5
        );
    }
    #[test]
    fn real_video_reports_intermediate_progress_and_reserves_completion() {
        let ffmpeg = tool("ffmpeg").expect("FFmpeg is required for media tests");
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("progress.mp4");
        let mut args = vec![];
        add(
            &mut args,
            &[
                "-v",
                "error",
                "-re",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x120:rate=30",
                "-t",
                "2",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-progress",
                "pipe:1",
                &output.to_string_lossy(),
            ],
        );
        let observed = Mutex::new(Vec::new());
        run(
            &ffmpeg,
            &args,
            &AtomicBool::new(false),
            &|percent, _| observed.lock().unwrap().push(percent),
            ProgressTotal::Frames(60.0),
            "Compressing",
            Some(Duration::from_secs(10)),
        )
        .unwrap();
        let values = observed.lock().unwrap();
        assert!(
            values.iter().any(|p| *p > 50.0 && *p < 90.0),
            "Missing intermediate progress: {values:?}"
        );
        assert!(values.windows(2).all(|pair| pair[0] <= pair[1]));
        assert!(values.iter().all(|p| *p < 100.0));
        assert_eq!(values.last(), Some(&99.0));
    }
    #[test]
    fn failed_encoder_does_not_report_completion() {
        let ffmpeg = tool("ffmpeg").expect("FFmpeg is required for media tests");
        let args = [
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=160x120:rate=4",
            "-t",
            "1",
            "-c:v",
            "missing_encoder",
            "-progress",
            "pipe:1",
            "-f",
            "null",
            "-",
        ]
        .map(String::from);
        let observed = Mutex::new(Vec::new());
        assert!(run(
            &ffmpeg,
            &args,
            &AtomicBool::new(false),
            &|percent, _| observed.lock().unwrap().push(percent),
            ProgressTotal::Frames(4.0),
            "Compressing",
            Some(Duration::from_secs(10))
        )
        .is_err());
        assert!(observed.lock().unwrap().iter().all(|p| *p < 99.0));
    }
    #[test]
    fn encoder_progress_reaches_completion_before_saving() {
        let ffmpeg = tool("ffmpeg").expect("FFmpeg is required for media tests");
        let tmp = tempfile::tempdir().unwrap();
        let output = tmp.path().join("progress.mp4");
        let mut args = vec![];
        add(
            &mut args,
            &[
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=160x120:rate=4",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-progress",
                "pipe:1",
                &output.to_string_lossy(),
            ],
        );
        let observed = Mutex::new(Vec::new());
        run(
            &ffmpeg,
            &args,
            &AtomicBool::new(false),
            &|percent, _| observed.lock().unwrap().push(percent),
            ProgressTotal::Seconds(1.0),
            "Compressing",
            None,
        )
        .unwrap();
        let values = observed.lock().unwrap();
        assert!(
            values.last().copied().unwrap_or(0.0) >= 99.0,
            "Encoding finished with reported progress: {values:?}"
        );
    }
    fn fixture(root: &Path, name: &str, input: &str, extras: &[&str]) -> PathBuf {
        let ffmpeg = tool("ffmpeg").expect("FFmpeg is required for media tests");
        let path = root.join(name);
        let mut args = vec![];
        add(&mut args, &["-v", "error", "-f", "lavfi", "-i", input]);
        add(&mut args, extras);
        args.push(path.to_string_lossy().into_owned());
        run(
            &ffmpeg,
            &args,
            &AtomicBool::new(false),
            &|_, _| {},
            ProgressTotal::Seconds(0.0),
            "Test",
            Some(Duration::from_secs(30)),
        )
        .unwrap();
        path
    }
    #[test]
    fn auto_image_retains_exact_pixels_and_never_expands_the_file() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "source.png",
            "testsrc=size=128x96:rate=1",
            &["-frames:v", "1"],
        );
        let result = compress(
            &source,
            &temp.path().join("auto.webp"),
            &json!({"compression_mode":"auto","image_format":"webp","target_mb":0.1,"image_quality":5,"max_height":480}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert!(n(&result["size"]) <= fs::metadata(&source).unwrap().len() as f64);
        if result["preserved_original"] != true {
            assert_eq!(
                decoded_hash(&source, "image", 1, &AtomicBool::new(false)).unwrap(),
                decoded_hash(
                    Path::new(result["path"].as_str().unwrap()),
                    "image",
                    1,
                    &AtomicBool::new(false)
                )
                .unwrap()
            );
            assert_eq!(result["width"], 128);
            assert_eq!(result["height"], 96);
        } else {
            assert_eq!(
                Path::new(result["path"].as_str().unwrap()),
                source.canonicalize().unwrap()
            );
        }
    }
    #[test]
    fn auto_audio_is_sample_exact_or_keeps_original() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "source.wav",
            "sine=frequency=440:sample_rate=44100",
            &["-t", "2"],
        );
        let result = compress(
            &source,
            &temp.path().join("auto.flac"),
            &json!({"compression_mode":"auto","audio_format":"flac","target_mb":0.1,"audio_sample_rate":22050}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert!(n(&result["size"]) <= fs::metadata(&source).unwrap().len() as f64);
        if result["preserved_original"] != true {
            assert_eq!(
                decoded_hash(&source, "audio", 1, &AtomicBool::new(false)).unwrap(),
                decoded_hash(
                    Path::new(result["path"].as_str().unwrap()),
                    "audio",
                    1,
                    &AtomicBool::new(false)
                )
                .unwrap()
            );
            assert_eq!(result["encoder"], "flac");
        }
    }
    #[test]
    fn auto_uses_selected_lossy_audio_and_image_formats() {
        let temp = tempfile::tempdir().unwrap();
        let audio = fixture(
            temp.path(),
            "source.wav",
            "sine=frequency=440:sample_rate=48000",
            &["-t", "2"],
        );
        let audio_output = temp.path().join("selected.mp3");
        let audio_result = compress(
            &audio,
            &audio_output,
            &json!({"compression_mode":"auto","audio_format":"mp3"}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_ne!(audio_result["preserved_original"], true);
        assert_eq!(probe(&audio_output).unwrap()["audio_codec"], "mp3");

        let image = fixture(
            temp.path(),
            "source.bmp",
            "testsrc2=size=512x512:rate=1",
            &["-frames:v", "1"],
        );
        let image_output = temp.path().join("selected.jpg");
        let image_result = compress(
            &image,
            &image_output,
            &json!({"compression_mode":"auto","image_format":"jpeg"}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_ne!(image_result["preserved_original"], true);
        assert_eq!(probe(&image_output).unwrap()["codec"], "mjpeg");
    }
    #[test]
    fn auto_keeps_small_original_instead_of_publishing_a_larger_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "tiny.mp3",
            "sine=frequency=440:sample_rate=44100",
            &["-t", "2", "-b:a", "8k"],
        );
        let destination = temp.path().join("tiny.flac");
        let result = compress(
            &source,
            &destination,
            &json!({"compression_mode":"auto","audio_format":"flac"}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result["preserved_original"], true);
        assert_eq!(
            Path::new(result["path"].as_str().unwrap()),
            source.canonicalize().unwrap()
        );
        assert!(!destination.exists());
    }
    #[test]
    fn auto_video_preserves_audio_stream_when_it_produces_a_copy() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "with-audio.mp4",
            "testsrc2=size=160x120:rate=12",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "8",
                "-c:a",
                "aac",
            ],
        );
        let result = compress(
            &source,
            &temp.path().join("with-audio.mkv"),
            &json!({"compression_mode":"auto","video_format":"mkv"}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        if result["preserved_original"] != true {
            let original = probe(&source).unwrap();
            let encoded = probe(Path::new(result["path"].as_str().unwrap())).unwrap();
            assert_eq!(encoded["audio_codec"], original["audio_codec"]);
            assert_eq!(
                decoded_hash(&source, "audio", 1, &AtomicBool::new(false)).unwrap(),
                decoded_hash(
                    Path::new(result["path"].as_str().unwrap()),
                    "audio",
                    1,
                    &AtomicBool::new(false)
                )
                .unwrap()
            );
        }
    }
    #[test]
    fn auto_mp4_converts_incompatible_audio() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "source.mkv",
            "testsrc2=size=160x120:rate=12",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=44100",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-c:a",
                "flac",
            ],
        );
        let info = probe(&source).unwrap();
        let options =
            settings::effective(&json!({"compression_mode":"auto","video_format":"mp4"})).unwrap();
        let output = temp.path().join("selected.mp4");
        encode_media(
            &source,
            &output,
            n(&info["size"]),
            &options,
            &info,
            &AtomicBool::new(false),
            &|_, _| {},
            None,
        )
        .unwrap();
        assert_eq!(probe(&output).unwrap()["audio_codec"], "aac");
    }
    #[test]
    fn auto_samples_reserve_quality_margin_before_full_encoding() {
        assert!(!sampled_quality_passes(&[(0.995, 0.989); 5]));
        assert!(sampled_quality_passes(&[(0.997, 0.990); 5]));
    }
    #[test]
    fn auto_comparison_rejects_short_and_long_outputs() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "source.mkv",
            "testsrc2=size=160x120:rate=12",
            &["-frames:v", "12", "-c:v", "libx264"],
        );
        let info = probe(&source).unwrap();
        for frames in [11, 13] {
            let output = fixture(
                temp.path(),
                &format!("output-{frames}.mkv"),
                "testsrc2=size=160x120:rate=12",
                &["-frames:v", &frames.to_string(), "-c:v", "libx264"],
            );
            let error = visual_score(
                &source,
                &output,
                &info,
                &AtomicBool::new(false),
                &|_, _| {},
                None,
                Some(12),
            )
            .unwrap_err();
            assert!(error.starts_with("Video frame counts changed"), "{error}");
        }
    }
    #[test]
    #[ignore = "Requires a local benchmark video in BLUBBERBOUND_BENCH_SOURCE"]
    fn auto_real_video_benchmark() {
        let source = PathBuf::from(std::env::var_os("BLUBBERBOUND_BENCH_SOURCE").unwrap());
        let temp = tempfile::tempdir().unwrap();
        let started = Instant::now();
        let previous = Mutex::new(String::new());
        let encodes = AtomicU64::new(0);
        let selection_seconds = AtomicU64::new(0);
        let result = compress(
            &source,
            &temp.path().join("result.mkv"),
            &json!({"compression_mode":"auto","video_format":"mkv"}),
            &AtomicBool::new(false),
            |_, stage| {
                let stage = stage.split(", ").next().unwrap();
                let mut previous = previous.lock().unwrap();
                if *previous != stage {
                    eprintln!("{:.2}s: {stage}", started.elapsed().as_secs_f64());
                    if stage.contains("encoding full file") {
                        if encodes.fetch_add(1, Ordering::Relaxed) == 0 {
                            selection_seconds.store(started.elapsed().as_secs(), Ordering::Relaxed);
                        }
                    }
                    *previous = stage.to_owned();
                }
            },
        )
        .unwrap();
        eprintln!(
            "Finished in {:.2}s: {result}",
            started.elapsed().as_secs_f64()
        );
        assert_ne!(result["preserved_original"], true);
        assert!(encodes.load(Ordering::Relaxed) <= 2);
        assert!(selection_seconds.load(Ordering::Relaxed) < 60);
        assert!(started.elapsed() < Duration::from_secs(360));
    }
    #[test]
    fn auto_sampling_only_runs_when_full_frame_work_is_large() {
        assert!(auto_sample_windows(8.0, 1920.0 * 1080.0 * 30.0).is_empty());
        assert!(auto_sample_windows(32.0, 320.0 * 240.0 * 24.0).is_empty());
        let windows = auto_sample_windows(120.0, 320.0 * 240.0 * 24.0);
        assert_eq!(windows.len(), 5);
        assert_eq!(windows[0], (11.925, 0.75));
        assert_eq!(windows[4], (107.325, 0.75));
        assert_eq!(auto_sample_windows(32.0, 1920.0 * 1080.0 * 30.0).len(), 5);
    }
    #[test]
    fn auto_sample_quality_is_judged_across_scenes() {
        let scores = [(0.997, 0.994), (0.994, 0.991), (0.999, 0.999)];
        assert!(sampled_quality_passes(&scores));
        assert!(sampled_quality_passes(&[
            (0.996, 0.990),
            (0.997, 0.990),
            (0.9966, 0.990)
        ]));
        assert!(!sampled_quality_passes(&[
            (0.997, 0.994),
            (0.992, 0.980),
            (0.999, 0.999)
        ]));
        assert!(!sampled_quality_passes(&[
            (0.992, 0.990),
            (0.992, 0.990),
            (0.992, 0.990)
        ]));
    }
    #[test]
    fn duplicate_frame_count_entries_are_accepted_when_equal() {
        assert_eq!(parse_frame_count("6462\r\n6462\r\n").unwrap(), 6462);
        assert!(parse_frame_count("6462\r\n6461\r\n").is_err());
        assert!(parse_frame_count("N/A\r\n").is_err());
        assert_eq!(
            source_frame_count(
                &json!({"frame_count":6462}),
                Path::new("nonexistent-video.mp4"),
                &AtomicBool::new(false)
            )
            .unwrap(),
            6462
        );
    }
    #[test]
    fn auto_visual_comparison_pairs_equal_frames_across_container_timebases() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "comparison-source.mp4",
            "testsrc2=size=320x240:rate=24",
            &[
                "-t",
                "8",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "10",
            ],
        );
        let output = fixture(
            temp.path(),
            "comparison-output.mkv",
            "testsrc2=size=320x240:rate=24",
            &[
                "-t", "8", "-c:v", "libx264", "-preset", "slow", "-crf", "25",
            ],
        );
        assert_eq!(
            counted_frames(&source, &AtomicBool::new(false)).unwrap(),
            192
        );
        assert_eq!(
            counted_frames(&output, &AtomicBool::new(false)).unwrap(),
            192
        );
        visual_score(
            &source,
            &output,
            &probe(&source).unwrap(),
            &AtomicBool::new(false),
            &|_, _| {},
            None,
            None,
        )
        .unwrap();
    }
    #[test]
    fn auto_video_search_avoids_repeated_full_file_encodes() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "search-source.mp4",
            "testsrc2=size=320x240:rate=24",
            &[
                "-t",
                "120",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "10",
            ],
        );
        let stages = Mutex::new(Vec::<String>::new());
        let start = Instant::now();
        let result = compress(
            &source,
            &temp.path().join("search-result.mkv"),
            &json!({"compression_mode":"auto","video_format":"mkv"}),
            &AtomicBool::new(false),
            |_, stage| {
                let stage = stage.split(", ").next().unwrap();
                let mut stages = stages.lock().unwrap();
                if stages.last().is_none_or(|previous| previous != stage) {
                    stages.push(stage.to_owned());
                }
            },
        )
        .unwrap();
        let full_encodes = stages
            .lock()
            .unwrap()
            .iter()
            .filter(|stage| {
                stage.starts_with("Auto quality: encoding candidate")
                    || stage.starts_with("Auto quality: encoding full file")
            })
            .count();
        assert!(
            full_encodes <= 2,
            "Auto encoded the full file {full_encodes} times in {:?}",
            start.elapsed()
        );
        if result["preserved_original"] != true {
            let score = visual_score(
                &source,
                Path::new(result["path"].as_str().unwrap()),
                &probe(&source).unwrap(),
                &AtomicBool::new(false),
                &|_, _| {},
                None,
                None,
            )
            .unwrap();
            assert!(score.0 >= 0.995 && score.1 >= 0.985);
        }
    }
    #[test]
    fn auto_stops_after_two_rejected_full_encodes() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "source.mp4",
            "testsrc2=size=320x240:rate=24",
            &[
                "-t",
                "120",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "10",
            ],
        );
        let mut info = probe(&source).unwrap();
        info["frame_count"] = json!(n(&info["frame_count"]) + 1.0);
        let previous = Mutex::new(String::new());
        let encodes = AtomicU64::new(0);
        let result = auto_candidate(
            &source,
            &temp.path().join("output.mkv"),
            &info,
            &settings::effective(&json!({"compression_mode":"auto","video_format":"mkv"})).unwrap(),
            &AtomicBool::new(false),
            &|_, stage| {
                let stage = stage.split(", ").next().unwrap();
                let mut previous = previous.lock().unwrap();
                if *previous != stage
                    && (stage.contains("encoding full file")
                        || stage.contains("encoding candidate"))
                {
                    encodes.fetch_add(1, Ordering::Relaxed);
                }
                *previous = stage.to_owned();
            },
        )
        .unwrap();
        assert!(result.is_none());
        assert_eq!(encodes.load(Ordering::Relaxed), 2);
    }
    #[test]
    fn auto_video_rejects_mismatched_frames_and_keeps_geometry() {
        let temp = tempfile::tempdir().unwrap();
        let source = fixture(
            temp.path(),
            "source.mp4",
            "testsrc2=size=160x120:rate=12",
            &[
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-crf",
                "10",
            ],
        );
        let wrong = fixture(
            temp.path(),
            "wrong.mkv",
            "color=black:size=160x120:rate=12",
            &["-t", "1", "-c:v", "libx264"],
        );
        let score = visual_score(
            &source,
            &wrong,
            &probe(&source).unwrap(),
            &AtomicBool::new(false),
            &|_, _| {},
            None,
            None,
        )
        .unwrap();
        assert!(score.0 < 0.995 || score.1 < 0.985);
        let result = compress(
            &source,
            &temp.path().join("auto.mkv"),
            &json!({"compression_mode":"auto","video_format":"mkv","target_mb":0.1,"scale_percent":50,"fps":6}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert!(n(&result["size"]) <= fs::metadata(&source).unwrap().len() as f64);
        if result["preserved_original"] != true {
            assert_eq!(result["width"], 160);
            assert_eq!(result["height"], 120);
            assert_eq!(
                counted_frames(&source, &AtomicBool::new(false)).unwrap(),
                counted_frames(
                    Path::new(result["path"].as_str().unwrap()),
                    &AtomicBool::new(false)
                )
                .unwrap()
            );
            let score = visual_score(
                &source,
                Path::new(result["path"].as_str().unwrap()),
                &probe(&source).unwrap(),
                &AtomicBool::new(false),
                &|_, _| {},
                None,
                None,
            )
            .unwrap();
            assert!(score.0 >= 0.995 && score.1 >= 0.985);
        }
    }
    #[test]
    fn real_image_quality_lossless_and_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.png",
            "testsrc=size=80x60",
            &["-frames:v", "1"],
        );
        let options =
            json!({"advanced_enabled":true,"rate_control":"quality","image_lossless":true});
        let out = tmp.path().join("out.webp");
        compress(&source, &out, &options, &AtomicBool::new(false), |_, _| {}).unwrap();
        let decoded = |p: &Path| {
            capture(
                &tool("ffmpeg").unwrap(),
                &[
                    "-v",
                    "error",
                    "-i",
                    p.to_str().unwrap(),
                    "-frames:v",
                    "1",
                    "-f",
                    "rawvideo",
                    "-pix_fmt",
                    "rgb24",
                    "-",
                ]
                .map(String::from),
                &AtomicBool::new(false),
                Duration::from_secs(10),
            )
            .unwrap()
        };
        assert_eq!(decoded(&source), decoded(&out));
        let preview_path = tmp.path().join("preview.webp");
        preview(
            &source,
            &preview_path,
            &options,
            &AtomicBool::new(false),
            |_, _| {},
            0.0,
            5.0,
        )
        .unwrap();
        assert_eq!(fs::read(&out).unwrap(), fs::read(preview_path).unwrap());
    }
    #[test]
    fn real_jpeg_white_transparency() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "transparent.png",
            "color=red@0:s=80x60,format=rgba",
            &["-frames:v", "1"],
        );
        let out = tmp.path().join("out.jpg");
        compress(
            &source,
            &out,
            &json!({"image_format":"jpeg"}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        let bytes = capture(
            &tool("ffmpeg").unwrap(),
            &[
                "-v",
                "error",
                "-i",
                out.to_str().unwrap(),
                "-frames:v",
                "1",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgb24",
                "-",
            ]
            .map(String::from),
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap();
        assert!(bytes.iter().all(|&v| v > 250));
    }
    #[test]
    fn real_audio_modes_and_clamped_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.wav",
            "sine=frequency=440:sample_rate=48000",
            &["-t", "2"],
        );
        let out = tmp.path().join("out.mp3");
        compress(
            &source,
            &out,
            &json!({"advanced_enabled":true,"audio_bitrate_kbps":64}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(n(&probe(&out).unwrap()["audio_bitrate"]), 64000.0);
        let sample = preview(
            &source,
            &tmp.path().join("preview.opus"),
            &json!({"audio_format":"opus"}),
            &AtomicBool::new(false),
            |_, _| {},
            1.5,
            5.0,
        )
        .unwrap();
        assert_eq!(sample["duration_seconds"], 0.5);
        assert!(compress(
            &source,
            &tmp.path().join("bad.opus"),
            &json!({"advanced_enabled":true,"audio_format":"opus","audio_sample_rate":44100}),
            &AtomicBool::new(false),
            |_, _| {}
        )
        .unwrap_err()
        .contains("Opus requires"));
    }
    #[test]
    fn original_resolution_survives_export_and_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.mkv",
            "testsrc=size=161x121:rate=10",
            &["-t", "2", "-c:v", "ffv1"],
        );
        let image = fixture(
            tmp.path(),
            "image.png",
            "testsrc=size=161x121",
            &["-frames:v", "1"],
        );
        {
            let options = json!({"target_mb":0.0001,"encoder":"software"});
            for (source, extension) in [(&source, "mp4"), (&image, "webp")] {
                let result = compress(
                    source,
                    &tmp.path().join(format!("output.{extension}")),
                    &options,
                    &AtomicBool::new(false),
                    |_, _| {},
                )
                .unwrap();
                assert_eq!(result["width"], 161, "{result}");
                assert_eq!(result["height"], 121, "{result}");
                assert!(result["warning"].is_string());
            }
            let sample = preview(
                &source,
                &tmp.path().join("preview.mp4"),
                &options,
                &AtomicBool::new(false),
                |_, _| {},
                0.0,
                1.0,
            )
            .unwrap();
            assert_eq!(sample["width"], 161);
            assert_eq!(sample["height"], 121);
        }
    }
    #[test]
    fn tiny_media_limits_save_playable_outputs() {
        let tmp = tempfile::tempdir().unwrap();
        let audio = fixture(
            tmp.path(),
            "source.wav",
            "sine=frequency=440:sample_rate=48000",
            &["-t", "1"],
        );
        let video = fixture(
            tmp.path(),
            "source.mp4",
            "testsrc2=size=160x120:rate=10",
            &["-t", "1", "-c:v", "libx264"],
        );
        for (kind, source, formats) in [
            ("audio", &audio, vec!["mp3", "opus", "m4a", "aac", "ogg"]),
            ("video", &video, vec!["mp4", "webm", "mkv", "mov"]),
        ] {
            for format in formats {
                let mut options = json!({"target_mb":0.0001,"encoder":"software"});
                options[format!("{kind}_format")] = json!(format);
                let output = tmp.path().join(format!("tiny.{format}"));
                let result = compress(
                    source,
                    &output,
                    &options,
                    &AtomicBool::new(false),
                    |_, _| {},
                )
                .unwrap();
                assert!(n(&result["size"]) > 100.0);
                assert!(s(&result, "warning").contains("size limit"));
                assert_eq!(probe(&output).unwrap()["kind"], kind);
            }
        }
    }
    #[test]
    fn additional_formats_export_and_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let audio = fixture(
            tmp.path(),
            "source.wav",
            "sine=frequency=440:sample_rate=48000",
            &["-t", "2"],
        );
        let video = fixture(
            tmp.path(),
            "source.mp4",
            "testsrc2=size=160x120:rate=10",
            &["-t", "2", "-c:v", "libx264"],
        );
        for (format, source, kind, codec) in [
            ("mkv", &video, "video", "h264"),
            ("mov", &video, "video", "h264"),
            ("m4a", &audio, "audio", "aac"),
            ("aac", &audio, "audio", "aac"),
            ("ogg", &audio, "audio", "vorbis"),
        ] {
            let mut options = json!({"advanced_enabled":true,"rate_control":"bitrate","video_bitrate_kbps":200,"encoder":"software"});
            options[format!("{kind}_format")] = json!(format);
            let output = tmp.path().join(format!("output.{format}"));
            compress(
                source,
                &output,
                &options,
                &AtomicBool::new(false),
                |_, _| {},
            )
            .unwrap();
            let info = probe(&output).unwrap();
            assert_eq!(info["kind"], kind);
            assert_eq!(
                info[if kind == "video" {
                    "codec"
                } else {
                    "audio_codec"
                }],
                codec
            );
            let sample = tmp.path().join(format!("preview.{format}"));
            preview(
                source,
                &sample,
                &options,
                &AtomicBool::new(false),
                |_, _| {},
                0.5,
                1.0,
            )
            .unwrap();
            assert!(sample.metadata().unwrap().len() > 0);
            options["rate_control"] = json!("target");
            options["target_mb"] = json!(0.06);
            let limited = tmp.path().join(format!("limited.{format}"));
            compress(
                source,
                &limited,
                &options,
                &AtomicBool::new(false),
                |_, _| {},
            )
            .unwrap();
            assert!(limited.metadata().unwrap().len() <= 60000);
        }
    }
    #[test]
    fn real_output_race_and_running_cancel() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.png",
            "testsrc=size=80x60",
            &["-frames:v", "1"],
        );
        let dest = tmp.path().join("out.webp");
        let result = compress(
            &source,
            &dest,
            &json!({}),
            &AtomicBool::new(false),
            |_, stage| {
                if stage == "Saving output" {
                    fs::write(&dest, b"keep").unwrap();
                }
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&dest).unwrap(), b"keep");
        let cancel = AtomicBool::new(false);
        let dest = tmp.path().join("cancel.webp");
        assert!(compress(&source, &dest, &json!({}), &cancel, |_, stage| {
            if stage == "Compressing image" {
                cancel.store(true, Ordering::Relaxed);
            }
        })
        .unwrap_err()
        .contains("cancelled"));
        assert!(!dest.exists());
        assert!(!fs::read_dir(tmp.path()).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".blubberbound-")));
    }
    #[test]
    fn image_metadata_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.jpg",
            "color=red:s=80x60",
            &["-frames:v", "1"],
        );
        let exif = b"II\x2a\0\x08\0\0\0\0\0\0\0\0\0";
        write_image_metadata(&source, exif, b"test-profile", 80, 60).unwrap();
        let retained = tmp.path().join("retained.webp");
        compress(
            &source,
            &retained,
            &json!({"advanced_enabled":true,"strip_metadata":false}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(
            image_metadata(&retained).unwrap(),
            (exif.to_vec(), b"test-profile".to_vec())
        );
        let stripped = tmp.path().join("stripped.webp");
        compress(
            &source,
            &stripped,
            &json!({}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(image_metadata(&stripped).unwrap(), (vec![], vec![]));
    }
    #[test]
    fn target_plan_uses_source_duration() {
        let i = json!({"kind":"video","size":500000,"duration":10,"has_audio":true,"width":640,"height":360,"fps":30});
        let p = plan(&i, 90000.0, &settings::defaults()).unwrap();
        assert!(n(&p["video_rate"]) < 70000.0);
    }
    #[test]
    fn real_target_fitting_copy_and_quality_preview() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.mp4",
            "testsrc2=size=640x360:rate=24",
            &["-t", "2", "-c:v", "libx264", "-preset", "ultrafast"],
        );
        let cancel = AtomicBool::new(false);
        let copy = tmp.path().join("copy.mp4");
        assert_eq!(
            compress(&source, &copy, &json!({}), &cancel, |_, _| {}).unwrap()["encoder"],
            "copy"
        );
        assert_eq!(fs::read(&source).unwrap(), fs::read(copy).unwrap());
        let result = compress(
            &source,
            &tmp.path().join("small.mp4"),
            &json!({"target_mb":0.045,"encoder":"software"}),
            &cancel,
            |_, _| {},
        )
        .unwrap();
        assert!(n(&result["size"]) <= 45000.0);
        assert!(
            (n(&probe(Path::new(result["path"].as_str().unwrap())).unwrap()["duration"]) - 2.0)
                .abs()
                < 0.15
        );
        let path = tmp.path().join("preview.mp4");
        let result = preview(&source, &path, &json!({}), &cancel, |_, _| {}, 0.5, 0.5).unwrap();
        assert_eq!(result["preserves_original"], true);
        let hash = |path: &Path, start: &str| {
            capture(
                &tool("ffmpeg").unwrap(),
                &[
                    "-v",
                    "error",
                    "-ss",
                    start,
                    "-i",
                    path.to_str().unwrap(),
                    "-t",
                    "0.5",
                    "-an",
                    "-f",
                    "framemd5",
                    "-",
                ]
                .map(String::from),
                &cancel,
                Duration::from_secs(10),
            )
            .unwrap()
        };
        let hashes = |bytes: Vec<u8>| {
            String::from_utf8(bytes)
                .unwrap()
                .lines()
                .filter(|line| !line.starts_with('#'))
                .map(|line| line.rsplit(',').next().unwrap().trim().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(hashes(hash(&source, "0.5")), hashes(hash(&path, "0")));
    }
    #[test]
    fn real_image_orientation() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "rotated.jpg",
            "color=blue:s=80x60",
            &["-frames:v", "1"],
        );
        let exif = b"II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
        write_image_metadata(&source, exif, &[], 80, 60).unwrap();
        let info = probe(&source).unwrap();
        assert_eq!((n(&info["width"]), n(&info["height"])), (60.0, 80.0));
        let result = compress(
            &source,
            &tmp.path().join("oriented.webp"),
            &json!({}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!((n(&result["width"]), n(&result["height"])), (60.0, 80.0));
    }
    #[test]
    fn real_webm_and_hardware_fallback() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.mp4",
            "testsrc2=size=320x180:rate=12",
            &["-t", "1", "-c:v", "libx264"],
        );
        for (format, settings) in [
            (
                "webm",
                json!({"video_format":"webm","advanced_enabled":true,"rate_control":"quality","crf":35}),
            ),
            (
                "mp4",
                json!({"advanced_enabled":true,"rate_control":"bitrate","encoder":"auto","video_bitrate_kbps":200}),
            ),
        ] {
            let output = tmp.path().join(format!("out.{format}"));
            compress(
                &source,
                &output,
                &settings,
                &AtomicBool::new(false),
                |_, _| {},
            )
            .unwrap();
            let info = probe(&output).unwrap();
            assert!((n(&info["duration"]) - 1.0).abs() < 0.15);
            assert_eq!(info["codec"], if format == "webm" { "vp9" } else { "h264" });
        }
    }
    #[test]
    fn real_animated_images_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.gif",
            "testsrc=size=40x30:rate=5",
            &["-t", "1"],
        );
        assert!(probe(&source).unwrap_err().contains("Animated"));
    }
    #[test]
    fn real_video_rotation_and_sample_aspect_ratio() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "wide.mp4",
            "testsrc2=size=320x180:rate=12",
            &["-t", "1", "-vf", "setsar=2", "-c:v", "libx264"],
        );
        let info = probe(&source).unwrap();
        assert_eq!((n(&info["width"]), n(&info["height"])), (640.0, 180.0));
        let rotated = tmp.path().join("rotated.mp4");
        let args = [
            "-v",
            "error",
            "-display_rotation",
            "90",
            "-i",
            source.to_str().unwrap(),
            "-c",
            "copy",
            rotated.to_str().unwrap(),
        ]
        .map(String::from);
        run(
            &tool("ffmpeg").unwrap(),
            &args,
            &AtomicBool::new(false),
            &|_, _| {},
            ProgressTotal::Seconds(0.0),
            "Test",
            None,
        )
        .unwrap();
        let info = probe(&rotated).unwrap();
        assert_eq!((n(&info["width"]), n(&info["height"])), (180.0, 640.0));
        let result = compress(
            &rotated,
            &tmp.path().join("out.mp4"),
            &json!({"advanced_enabled":true,"rate_control":"quality","output_height":320}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!((n(&result["width"]), n(&result["height"])), (90.0, 320.0));
        let output = probe(Path::new(result["path"].as_str().unwrap())).unwrap();
        assert_eq!((n(&output["width"]), n(&output["height"])), (90.0, 320.0));
    }
    #[test]
    fn real_video_metadata_and_manual_audio() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "tagged.mp4",
            "testsrc2=size=160x90:rate=12",
            &[
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-t",
                "1",
                "-c:v",
                "libx264",
                "-c:a",
                "aac",
                "-metadata",
                "title=Test title",
                "-metadata:s:a:0",
                "language=deu",
            ],
        );
        for strip in [true, false] {
            let output = tmp.path().join(format!("{strip}.mp4"));
            compress(&source,&output,&json!({"advanced_enabled":true,"rate_control":"quality","strip_metadata":strip,"audio_channels":1,"audio_sample_rate":44100}),&AtomicBool::new(false),|_,_|{}).unwrap();
            let info = probe(&output).unwrap();
            assert_eq!(n(&info["audio_channels"]), 1.0);
            assert_eq!(n(&info["audio_sample_rate"]), 44100.0);
            let data: Value = serde_json::from_slice(
                &capture(
                    &tool("ffprobe").unwrap(),
                    &[
                        "-v",
                        "error",
                        "-show_entries",
                        "format_tags=title:stream_tags=language",
                        "-of",
                        "json",
                        output.to_str().unwrap(),
                    ]
                    .map(String::from),
                    &AtomicBool::new(false),
                    Duration::from_secs(10),
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                data["format"]["tags"]["title"].as_str(),
                if strip { None } else { Some("Test title") }
            );
            assert_eq!(
                data["streams"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|v| v["tags"]["language"] == "deu"),
                !strip
            );
        }
    }
    #[test]
    fn bounding_box_and_frame_rate_preserve_aspect() {
        let info = json!({"width":1920,"height":1080,"fps":60});
        for (options, expected) in [
            (
                json!({"advanced_enabled":true,"output_width":1000,"output_height":300}),
                (532, 300),
            ),
            (
                json!({"advanced_enabled":true,"scale_percent":150,"output_width":3000}),
                (2880, 1620),
            ),
            (json!({"max_height":480}), (852, 480)),
        ] {
            assert_eq!(
                dimensions(&info, &settings::effective(&options).unwrap(), 0.0, true),
                expected
            );
        }
    }
    #[test]
    fn failed_media_probe_preserves_files() {
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("broken.mp4");
        let output = tmp.path().join("out.mp4");
        fs::write(&source, b"broken input").unwrap();
        assert!(compress(
            &source,
            &output,
            &json!({}),
            &AtomicBool::new(false),
            |_, _| {}
        )
        .is_err());
        assert!(!output.exists());
        assert_eq!(fs::read(source).unwrap(), b"broken input");
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }
    #[test]
    fn real_image_target_fitting_and_fixed_geometry() {
        let tmp = tempfile::tempdir().unwrap();
        let source = fixture(
            tmp.path(),
            "source.png",
            "testsrc2=size=400x300,noise=alls=100:allf=t",
            &["-frames:v", "1"],
        );
        let result = compress(
            &source,
            &tmp.path().join("fit.webp"),
            &json!({"target_mb":0.006,"max_height":480}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert!(n(&result["size"]) <= 6000.0);
        assert!(n(&result["width"]) < 400.0);
        let fixed = compress(
            &source,
            &tmp.path().join("fixed.webp"),
            &json!({"advanced_enabled":true,"target_mb":0.0001,"output_width":200}),
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert!(n(&fixed["size"]) > 100.0);
        assert_eq!(n(&fixed["width"]), 200.0);
        assert!(s(&fixed, "warning").contains("size limit"));
        assert!(tmp.path().join("fixed.webp").is_file());
    }
    #[test]
    fn explicit_geometry_is_not_reduced() {
        let o = settings::effective(&json!({"advanced_enabled":true,"scale_percent":50})).unwrap();
        assert_eq!(
            dimensions(
                &json!({"width":640,"height":360,"fps":24}),
                &o,
                12000.0,
                true
            ),
            (320, 180)
        );
    }
    #[test]
    fn cancelled_export_preserves_destination() {
        let temp = tempfile::tempdir().unwrap();
        let dest = temp.path().join("out.mp4");
        fs::write(&dest, b"keep").unwrap();
        assert!(compress(&dest, &dest, &json!({}), &AtomicBool::new(true), |_, _| {}).is_err());
        assert_eq!(fs::read(dest).unwrap(), b"keep");
    }
    #[test]
    fn real_video_preview_and_safe_publication() {
        let Some(ffmpeg) = tool("ffmpeg") else { return };
        let tmp = tempfile::tempdir().unwrap();
        let source = tmp.path().join("source.mp4");
        let args = [
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=24",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            source.to_str().unwrap(),
        ]
        .map(String::from);
        run(
            &ffmpeg,
            &args,
            &AtomicBool::new(false),
            &|_, _| {},
            ProgressTotal::Seconds(2.0),
            "Test",
            None,
        )
        .unwrap();
        let original = fs::read(&source).unwrap();
        let options = json!({"advanced_enabled":true,"rate_control":"bitrate","video_bitrate_kbps":200,"scale_percent":50,"encoder":"software"});
        let output = tmp.path().join("out.mp4");
        let result = compress(
            &source,
            &output,
            &options,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result["width"], 160);
        assert_eq!(result["height"], 90);
        assert!(b(&probe(&output).unwrap(), "has_audio"));
        let sample = preview(
            &source,
            &tmp.path().join("preview.mp4"),
            &options,
            &AtomicBool::new(false),
            |_, _| {},
            0.5,
            0.5,
        )
        .unwrap();
        assert_eq!(sample["duration_seconds"], 0.5);
        assert!(compress(
            &source,
            &output,
            &options,
            &AtomicBool::new(false),
            |_, _| {}
        )
        .is_err());
        assert_eq!(fs::read(source).unwrap(), original);
    }
}
