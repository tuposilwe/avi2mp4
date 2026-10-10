//! Conversion engine shared by the command line and the GUI.
//!
//! Strategy per file:
//!   * if a stream's codec is already valid for the target format -> copy it (lossless, fast)
//!   * otherwise re-encode it with the target format's standard codec
//!
//! Requires `ffmpeg` and `ffprobe` on PATH (or set FFMPEG_DIR to their folder).

use std::env;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq)]
pub enum Format {
    Mp4,
    Mov,
    Mkv,
    Webm,
    Avi,
    Gif,
    Mp3,
    M4a,
    Wav,
    Flac,
    Ogg,
    Opus,
}

pub const FORMATS: &[(&str, Format)] = &[
    ("mp4", Format::Mp4),
    ("mov", Format::Mov),
    ("mkv", Format::Mkv),
    ("webm", Format::Webm),
    ("avi", Format::Avi),
    ("gif", Format::Gif),
    ("mp3", Format::Mp3),
    ("m4a", Format::M4a),
    ("wav", Format::Wav),
    ("flac", Format::Flac),
    ("ogg", Format::Ogg),
    ("opus", Format::Opus),
];

/// Extensions picked up when a directory is given (unless --from overrides it).
pub const INPUT_EXTS: &[&str] = &[
    "avi", "mkv", "mp4", "m4v", "mov", "wmv", "flv", "webm", "mpg", "mpeg", "ts", "mts", "m2ts",
    "3gp", "vob", "ogv", "mp3", "wav", "flac", "m4a", "aac", "ogg", "opus", "wma",
];

pub const PRESETS: &[&str] = &[
    "ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow",
];

impl Format {
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim_start_matches('.');
        FORMATS
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(s))
            .map(|(_, f)| *f)
    }

    pub fn ext(self) -> &'static str {
        FORMATS.iter().find(|(_, f)| *f == self).unwrap().0
    }

    pub fn is_audio(self) -> bool {
        use Format::*;
        matches!(self, Mp3 | M4a | Wav | Flac | Ogg | Opus)
    }

    /// Video codecs that can be stored in this container without re-encoding.
    fn copies_video(self, codec: &str) -> bool {
        use Format::*;
        match self {
            Mp4 => matches!(codec, "h264" | "hevc"),
            Mov => matches!(codec, "h264" | "hevc" | "prores" | "mjpeg"),
            Mkv => true,
            Webm => matches!(codec, "vp8" | "vp9" | "av1"),
            Avi => matches!(codec, "h264" | "mpeg4" | "msmpeg4v3" | "mjpeg" | "utvideo" | "huffyuv"),
            _ => false,
        }
    }

    /// Audio codecs that can be stored in this container without re-encoding.
    fn copies_audio(self, codec: &str) -> bool {
        use Format::*;
        match self {
            Mp4 => matches!(codec, "aac" | "mp3"),
            Mov => matches!(codec, "aac" | "mp3" | "alac" | "pcm_s16le"),
            M4a => matches!(codec, "aac" | "alac"),
            Mkv => true,
            Webm | Opus => matches!(codec, "opus") || (self == Webm && codec == "vorbis"),
            Avi => matches!(codec, "mp3" | "ac3" | "pcm_s16le"),
            Mp3 => codec == "mp3",
            Ogg => matches!(codec, "vorbis" | "opus" | "flac"),
            Wav => matches!(codec, "pcm_s16le" | "pcm_s24le" | "pcm_f32le"),
            Flac => codec == "flac",
            Gif => false,
        }
    }

    fn video_encoder(self) -> &'static str {
        match self {
            Format::Gif => "gif",
            Format::Webm => "vp9",
            _ => "h264",
        }
    }

    /// (display name, ffmpeg args) for the audio encoder used by this format.
    fn audio_encoder(self, bitrate: &str) -> (&'static str, Vec<String>) {
        use Format::*;
        let args = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        match self {
            Mp4 | Mov | M4a | Mkv => ("aac", args(&["-c:a", "aac", "-b:a", bitrate])),
            Webm | Opus => ("opus", args(&["-c:a", "libopus", "-b:a", bitrate])),
            Avi | Mp3 => ("mp3", args(&["-c:a", "libmp3lame", "-b:a", bitrate])),
            Ogg => ("vorbis", args(&["-c:a", "libvorbis", "-b:a", bitrate])),
            Wav => ("pcm", args(&["-c:a", "pcm_s16le"])),
            Flac => ("flac", args(&["-c:a", "flac"])),
            Gif => ("none", Vec::new()),
        }
    }
}

#[derive(Clone)]
pub struct Options {
    pub inputs: Vec<PathBuf>,
    pub out_dir: Option<PathBuf>,
    pub format: Format,
    pub from: Option<Vec<String>>,
    pub recursive: bool,
    pub crf: Option<u8>,
    pub preset: String,
    pub overwrite: bool,
    pub force_encode: bool,
    pub width: Option<u32>,
    pub fps: Option<f64>,
    pub start: Option<String>,
    pub duration: Option<String>,
    pub no_audio: bool,
    pub audio_bitrate: String,
    /// Set this to abort the running conversion (used by the GUI).
    pub cancel: Option<Arc<AtomicBool>>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            inputs: Vec::new(),
            out_dir: None,
            format: Format::Mp4,
            from: None,
            recursive: false,
            crf: None,
            preset: "medium".into(),
            overwrite: false,
            force_encode: false,
            width: None,
            fps: None,
            start: None,
            duration: None,
            no_audio: false,
            audio_bitrate: "192k".into(),
            cancel: None,
        }
    }
}

/// Updates sent by [`convert`] while it works.
pub enum Report {
    /// What happens to each stream, e.g. "video: h264 (copy), audio: mp3 (encode aac)".
    Streams(String),
    /// Percentage done, 0-100.
    Progress(f64),
}

/// Parses "90", "1:30" or "01:02:03.5" into seconds.
pub fn parse_time(s: &str) -> Option<f64> {
    s.split(':')
        .try_fold(0.0, |acc, part| part.parse::<f64>().ok().map(|v| acc * 60.0 + v))
        .filter(|t| *t >= 0.0)
}

fn tool(name: &str) -> PathBuf {
    match env::var_os("FFMPEG_DIR") {
        Some(dir) => Path::new(&dir).join(name),
        None => PathBuf::from(name),
    }
}

/// A `Command` for ffmpeg/ffprobe. When running without a console (the GUI was
/// double-clicked), it stops Windows from flashing a console window per call.
fn command(name: &str) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(tool(name));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetConsoleWindow() -> *mut std::ffi::c_void;
        }
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        if unsafe { GetConsoleWindow() }.is_null() {
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
    }
    cmd
}

pub fn check_tools() -> Result<(), String> {
    check_tool("ffmpeg").and_then(|_| check_tool("ffprobe"))
}

fn check_tool(name: &str) -> Result<(), String> {
    command(name)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|_| ())
        .map_err(|_| {
            format!("`{name}` not found. Install ffmpeg (e.g. `winget install Gyan.FFmpeg`) or set FFMPEG_DIR.")
        })
}

fn extension(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn wanted(p: &Path, opts: &Options) -> bool {
    let ext = extension(p);
    match &opts.from {
        Some(list) => list.contains(&ext),
        None => INPUT_EXTS.contains(&ext.as_str()) && ext != opts.format.ext(),
    }
}

fn scan_dir(dir: &Path, opts: &Options, out: &mut Vec<PathBuf>) -> Result<(), String> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if opts.recursive {
                scan_dir(&path, opts, out)?;
            }
        } else if wanted(&path, opts) {
            out.push(path);
        }
    }
    Ok(())
}

pub fn collect_files(opts: &Options) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    for input in &opts.inputs {
        if input.is_dir() {
            scan_dir(input, opts, &mut files)?;
        } else if input.is_file() {
            files.push(input.clone());
        } else {
            return Err(format!("{} does not exist", input.display()));
        }
    }
    Ok(files)
}

/// Runs ffprobe and returns its trimmed stdout (first line).
fn probe(file: &Path, args: &[&str]) -> Option<String> {
    let out = command("ffprobe")
        .args(["-v", "error"])
        .args(args)
        .args(["-of", "csv=p=0"])
        .arg(file)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s.lines().next()?.trim().trim_end_matches(',').to_string();
    (!line.is_empty()).then_some(line)
}

/// Codec of the first real video stream; album-art images in audio files don't count.
fn video_codec(file: &Path) -> Option<String> {
    let line = probe(
        file,
        &["-select_streams", "v:0", "-show_entries", "stream=codec_name:stream_disposition=attached_pic"],
    )?;
    let mut fields = line.split(',');
    let codec = fields.next()?.to_string();
    let attached_pic = fields.next() == Some("1");
    (!attached_pic).then_some(codec)
}

fn audio_codec(file: &Path) -> Option<String> {
    probe(file, &["-select_streams", "a:0", "-show_entries", "stream=codec_name"])
}

fn duration_secs(file: &Path) -> Option<f64> {
    probe(file, &["-show_entries", "format=duration"])?.parse().ok()
}

pub fn output_path(input: &Path, opts: &Options) -> PathBuf {
    let name = input.with_extension(opts.format.ext());
    let out = match &opts.out_dir {
        Some(dir) => dir.join(name.file_name().unwrap()),
        None => name,
    };
    // Never write over the input itself (e.g. mp4 -> mp4 re-encode).
    let same = out.exists()
        && std::fs::canonicalize(&out).ok() == std::fs::canonicalize(input).ok();
    if same {
        let stem = input.file_stem().unwrap().to_string_lossy();
        out.with_file_name(format!("{stem}_converted.{}", opts.format.ext()))
    } else {
        out
    }
}

/// libvpx `-cpu-used` value matching an x264-style preset name.
fn vp9_speed(preset: &str) -> &'static str {
    match preset {
        "ultrafast" | "superfast" | "veryfast" => "5",
        "faster" | "fast" => "4",
        "medium" => "2",
        "slow" => "1",
        _ => "0",
    }
}

fn describe(codec: &Option<String>, copy: bool, encoder: &str) -> String {
    match codec {
        None => "none".into(),
        Some(c) if copy => format!("{c} (copy)"),
        Some(c) => format!("{c} (encode {encoder})"),
    }
}

pub fn convert(
    input: &Path,
    opts: &Options,
    report: &mut dyn FnMut(Report),
) -> Result<Option<PathBuf>, String> {
    let fmt = opts.format;
    let output = output_path(input, opts);
    if output.exists() && !opts.overwrite {
        return Ok(None);
    }

    let vcodec = if fmt.is_audio() { None } else { video_codec(input) };
    let acodec = if opts.no_audio || fmt == Format::Gif { None } else { audio_codec(input) };

    if fmt == Format::Gif && vcodec.is_none() {
        return Err("no video stream to make a GIF from".into());
    }
    if fmt.is_audio() && acodec.is_none() {
        return Err("no audio stream found".into());
    }
    if vcodec.is_none() && acodec.is_none() {
        return Err("no usable audio or video streams found (corrupt or not a media file?)".into());
    }

    let must_encode = opts.force_encode || opts.width.is_some() || opts.fps.is_some();
    let copy_video = !must_encode && vcodec.as_deref().is_some_and(|c| fmt.copies_video(c));
    let copy_audio = acodec.as_deref().is_some_and(|c| fmt.copies_audio(c));
    let (aenc_name, aenc_args) = fmt.audio_encoder(&opts.audio_bitrate);

    report(Report::Streams(format!(
        "video: {}, audio: {}",
        describe(&vcodec, copy_video, fmt.video_encoder()),
        describe(&acodec, copy_audio, aenc_name),
    )));

    let mut cmd = command("ffmpeg");
    cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error", "-y"]);
    if let Some(start) = &opts.start {
        cmd.args(["-ss", start]);
    }
    cmd.arg("-i").arg(input);
    if let Some(dur) = &opts.duration {
        cmd.args(["-t", dur]);
    }
    if vcodec.is_some() {
        cmd.args(["-map", "0:v:0"]);
    }
    if acodec.is_some() {
        cmd.args(["-map", "0:a:0"]);
    }

    if let Some(vc) = &vcodec {
        if copy_video {
            cmd.args(["-c:v", "copy"]);
            if vc == "hevc" && matches!(fmt, Format::Mp4 | Format::Mov) {
                cmd.args(["-tag:v", "hvc1"]); // plays in QuickTime/Apple players
            }
        } else {
            let mut filters: Vec<String> = Vec::new();
            match fmt {
                Format::Gif => {
                    let fps = opts.fps.unwrap_or(12.0);
                    let width = opts.width.unwrap_or(480);
                    filters.push(format!("fps={fps}"));
                    filters.push(format!("scale={width}:-1:flags=lanczos"));
                    // Two-pass palette in one filter graph gives far better GIF colours.
                    filters.push("split[a][b];[a]palettegen[p];[b][p]paletteuse".into());
                }
                Format::Webm => {
                    let crf = opts.crf.unwrap_or(31).to_string();
                    cmd.args(["-c:v", "libvpx-vp9", "-crf", &crf, "-b:v", "0"])
                        .args(["-deadline", "good", "-cpu-used", vp9_speed(&opts.preset)])
                        .args(["-row-mt", "1", "-pix_fmt", "yuv420p"]);
                }
                _ => {
                    let crf = opts.crf.unwrap_or(20).to_string();
                    cmd.args(["-c:v", "libx264", "-preset", &opts.preset, "-crf", &crf])
                        .args(["-pix_fmt", "yuv420p"]);
                }
            }
            if fmt != Format::Gif {
                if let Some(fps) = opts.fps {
                    filters.push(format!("fps={fps}"));
                }
                // -2 / trunc keep dimensions even, which yuv420p requires.
                filters.push(match opts.width {
                    Some(w) => format!("scale={w}:-2"),
                    None => "scale=trunc(iw/2)*2:trunc(ih/2)*2".into(),
                });
            }
            cmd.arg("-vf").arg(filters.join(","));
        }
    }

    if acodec.is_some() {
        if copy_audio {
            cmd.args(["-c:a", "copy"]);
        } else {
            cmd.args(&aenc_args);
        }
    }

    if matches!(fmt, Format::Mp4 | Format::Mov | Format::M4a) {
        cmd.args(["-movflags", "+faststart"]);
    }

    cmd.args(["-progress", "pipe:1", "-nostats"])
        .arg(&output)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("failed to start ffmpeg: {e}"))?;

    // Collect stderr on a separate thread so ffmpeg never blocks on a full pipe.
    let stderr = child.stderr.take().unwrap();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        let _ = std::io::Read::read_to_string(&mut BufReader::new(stderr), &mut s);
        s
    });

    // Length of the part actually being converted, for the progress percentage.
    let start = opts.start.as_deref().and_then(parse_time).unwrap_or(0.0);
    let duration = duration_secs(input)
        .map(|d| (d - start).max(0.0))
        .map(|d| match opts.duration.as_deref().and_then(parse_time) {
            Some(limit) => d.min(limit),
            None => d,
        })
        .filter(|d| *d > 0.0);

    let stdout = child.stdout.take().unwrap();
    let cancelled = || opts.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed));
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        if cancelled() {
            let _ = child.kill();
            break;
        }
        if let Some(us) = line.strip_prefix("out_time_us=") {
            if let (Ok(us), Some(total)) = (us.parse::<f64>(), duration) {
                report(Report::Progress((us / 1_000_000.0 / total * 100.0).clamp(0.0, 100.0)));
            }
        }
    }

    let status = child.wait().map_err(|e| e.to_string())?;
    let errors = err_thread.join().unwrap_or_default();

    if cancelled() {
        let _ = std::fs::remove_file(&output);
        return Err("cancelled".into());
    }
    if !status.success() {
        let _ = std::fs::remove_file(&output);
        let msg = errors.trim();
        return Err(if msg.is_empty() {
            format!("ffmpeg exited with {status}")
        } else {
            msg.to_string()
        });
    }
    Ok(Some(output))
}
