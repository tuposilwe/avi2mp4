# avi2mp4

A small command-line tool, written in Rust, that converts video and audio files between formats.
By default it turns `.avi` files into `.mp4`, but it can also:

- convert any common video to **MP4, MKV, WebM, MOV or AVI**
- make **animated GIFs**
- extract the sound as **MP3, M4A, WAV, FLAC, OGG or Opus**
- **resize**, change the **frame rate**, **trim** a section, or **remove the audio**
- convert whole folders, including subfolders

It always picks the fastest conversion that works. If a stream is already in a codec the target format
supports, it is **copied** unchanged (lossless, takes seconds). Otherwise it is **re-encoded**.

The actual encoding is done by [ffmpeg](https://ffmpeg.org). avi2mp4 decides which settings to use, runs ffmpeg,
shows progress and handles batches of files. It has no Rust dependencies beyond the standard library.

---

## Contents

1. [Setup (one time)](#1-setup-one-time)
2. [Build the tool](#2-build-the-tool)
3. [Quick start](#3-quick-start)
4. [Supported formats](#4-supported-formats)
5. [Conversion recipes](#5-conversion-recipes)
6. [All options](#6-all-options)
7. [Quality and speed settings](#7-quality-and-speed-settings)
8. [What happens during a conversion](#8-what-happens-during-a-conversion)
9. [How long it takes](#9-how-long-it-takes)
10. [Troubleshooting](#10-troubleshooting)
11. [Project layout](#11-project-layout)
12. [Uninstalling](#12-uninstalling)

---

## 1. Setup (one time)

You need two things: **ffmpeg**, which does the conversion, and **Rust**, which compiles the tool.
Run all commands below in **PowerShell**.

### 1.1 Install ffmpeg

```powershell
winget install --id Gyan.FFmpeg -e --source winget
```

> `--source winget` matters. Without it, winget also searches the Microsoft Store source, and on some
> machines that fails with `0x8a15005e : The server certificate did not match any of the expected values`.

The download is fairly large and can take 10 minutes or more on a slow connection. This "full" build includes every
encoder the tool uses (x264, VP9, Opus, Vorbis, MP3).

**Close and reopen PowerShell** afterwards so the new `PATH` is picked up. Then check it:

```powershell
ffmpeg -version
ffprobe -version
```

### 1.2 Install Rust

The simplest option on Windows is the **GNU toolchain**. It includes its own linker, so you do *not* need the
multi-gigabyte Visual Studio Build Tools.

```powershell
Invoke-WebRequest https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-gnu/rustup-init.exe -OutFile rustup-init.exe
.\rustup-init.exe -y --default-host x86_64-pc-windows-gnu --profile minimal
Remove-Item rustup-init.exe
```

**Close and reopen PowerShell**, then check it:

```powershell
cargo --version
```

> If you already have Visual Studio with the "Desktop development with C++" workload, the standard installer
> from <https://rustup.rs> (MSVC toolchain) works too.

---

## 2. Build the tool

```powershell
cd C:\Users\MasterMind47\Videos\avi2mp4
cargo build --release
```

The first build takes a minute or two. The program ends up at:

```
C:\Users\MasterMind47\Videos\avi2mp4\target\release\avi2mp4.exe
```

### Optional: run it from any folder

To type plain `avi2mp4` instead of the full path:

```powershell
cargo install --path C:\Users\MasterMind47\Videos\avi2mp4
```

The examples below assume you did this. If you didn't, replace `avi2mp4` with the full path to `avi2mp4.exe`.
**Run `cargo install` again after changing the source code**, or the old version stays installed.

---

## 3. Quick start

```powershell
avi2mp4 video.avi                          # -> video.mp4
avi2mp4 video.avi -f webm                  # -> video.webm
avi2mp4 video.mkv -f mp3                   # -> video.mp3 (sound only)
avi2mp4 video.mp4 -f gif -s 0:30 -d 5      # 5-second GIF starting at 0:30
avi2mp4 C:\Users\MasterMind47\Videos       # every video in the folder -> mp4
```

- The output goes next to the input with the new extension, unless you use `-o <folder>`.
- **The original file is never modified or deleted.**
- If the output would have the same name as the input (e.g. re-encoding an MP4 to MP4), it is named
  `<name>_converted.<ext>` instead.
- Existing output files are **skipped** unless you add `-y`.

### Example output

```
[1/1] C:\Users\MasterMind47\Videos\terraforn.avi -> mp4
    video: utvideo (encode h264), audio: pcm_s16le (encode aac)
    progress: 100.0%
    -> C:\Users\MasterMind47\Videos\terraforn.mp4

Done: 1 converted, 0 skipped, 0 failed.
```

The second line shows the codecs inside the input and what happens to each one: `copy` or `encode <codec>`.

---

## 4. Supported formats

### Input

Any file ffmpeg can read works when you name it directly. When you pass a **folder**, these extensions are picked up:

```
video: avi mkv mp4 m4v mov wmv flv webm mpg mpeg ts mts m2ts 3gp vob ogv
audio: mp3 wav flac m4a aac ogg opus wma
```

Files already in the target format are left out of folder scans. For example, `-f mp4` doesn't pick up the
existing `.mp4` files. Use `--from` to choose exactly which extensions to scan.

### Output: video

Choose the output with `-f <format>`. The default is `mp4`.

| Format | Best for | Copied without re-encoding | Otherwise encoded to |
|---|---|---|---|
| `mp4` | Everything: phones, browsers, TVs, editors, sharing | H.264, HEVC / AAC, MP3 | H.264 + AAC |
| `mov` | Apple devices, Final Cut, iMovie | H.264, HEVC, ProRes, MJPEG / AAC, MP3, ALAC, PCM | H.264 + AAC |
| `mkv` | Archiving; holds almost any codec | **Everything** | H.264 + AAC |
| `webm` | Websites, open formats | VP8, VP9, AV1 / Opus, Vorbis | VP9 + Opus |
| `avi` | Old software and players | H.264, MPEG-4, MJPEG, UtVideo, HuffYUV / MP3, AC3, PCM | H.264 + MP3 |
| `gif` | Short silent loops for chats and docs | — | GIF (12 fps, 480 px wide by default) |

Because MKV accepts every codec, **converting to MKV is nearly always an instant, lossless copy**, even from
huge lossless AVI recordings. The file stays big, though. Use MP4 if you want a small file.

### Output: audio only

The video is dropped and only the first audio track is kept.

| Format | Type | Copied without re-encoding | Otherwise encoded to |
|---|---|---|---|
| `mp3` | Lossy, plays everywhere | MP3 | MP3 (192 kbps) |
| `m4a` | Lossy, Apple/iTunes friendly | AAC, ALAC | AAC (192 kbps) |
| `ogg` | Lossy, open format | Vorbis, Opus, FLAC | Vorbis (192 kbps) |
| `opus` | Lossy, best quality per size | Opus | Opus (192 kbps) |
| `flac` | Lossless, compressed | FLAC | FLAC |
| `wav` | Lossless, uncompressed (large) | PCM 16/24-bit, 32-bit float | PCM 16-bit |

Change the lossy bitrate with `-b`, e.g. `-b 320k` or `-b 128k`.

---

## 5. Conversion recipes

### Convert all AVI recordings to MP4

```powershell
avi2mp4 C:\Users\MasterMind47\Videos
```

### Only convert certain types from a folder

```powershell
avi2mp4 C:\Users\MasterMind47\Videos --from avi,wmv,flv
```

### Include subfolders and collect the results in one place

```powershell
avi2mp4 D:\Recordings -r -o D:\Converted
```

All outputs go flat into `D:\Converted`. If two subfolders contain files with the same name, the second one is
skipped (or overwrites the first if you use `-y`).

### Make a file smaller for sending

```powershell
avi2mp4 clip.mp4 -e -w 1280 -c 26 -p fast
```

`-e` forces a re-encode, `-w 1280` scales the video down to 1280 px wide, and `-c 26` lowers the quality a bit.

### Make a video for a website

```powershell
avi2mp4 clip.avi -f webm
```

### Extract the soundtrack

```powershell
avi2mp4 clip.avi -f mp3 -b 320k     # high-quality MP3
avi2mp4 clip.avi -f flac            # lossless
```

### Cut out a section

```powershell
avi2mp4 long.avi -s 1:30 -d 20      # 20 seconds starting at 1:30
avi2mp4 long.avi -s 0:01:30.5       # from 1:30.5 to the end
```

Times can be written as seconds (`90`), `MM:SS` (`1:30`) or `HH:MM:SS` (`0:01:30`), each with optional decimals.

> **Trimming and copying:** when the video is *copied* rather than re-encoded, the cut can only start on a keyframe.
> The clip may then begin a few seconds early and run longer than asked. In testing, a 5-second copy came out
> at 7.2 seconds. Add `-e` for frame-exact cuts.

### Make a GIF

```powershell
avi2mp4 clip.mp4 -f gif -s 0:10 -d 4               # 4 s, 480 px wide, 12 fps
avi2mp4 clip.mp4 -f gif -s 0:10 -d 4 -w 320 --fps 10  # smaller file
```

GIFs get big quickly, so keep them to a few seconds. Lower `-w` and `--fps` make them much smaller.

### Remove the audio

```powershell
avi2mp4 clip.avi -a
```

### Change the frame rate

```powershell
avi2mp4 clip.avi --fps 30
```

### Lossless repackage to MKV (instant)

```powershell
avi2mp4 C:\Users\MasterMind47\Videos -f mkv --from avi
```

### Re-encode an MP4 in place

```powershell
avi2mp4 clip.mp4 -e
```

This writes `clip_converted.mp4`. The original is kept.

---

## 6. All options

```
avi2mp4 [OPTIONS] <FILE|DIR>...
```

Options can come before or after the file names, in any order.

### Output format

| Option | Meaning | Default |
|---|---|---|
| `-f`, `--format <FMT>` | `mp4` `mov` `mkv` `webm` `avi` `gif` `mp3` `m4a` `wav` `flac` `ogg` `opus` | `mp4` |

### Choosing input files

| Option | Meaning | Default |
|---|---|---|
| `--from <EXT,...>` | Only pick these extensions when scanning folders, e.g. `--from avi,wmv` | All known media types |
| `-r`, `--recursive` | Also scan subfolders | Off |

### Output location

| Option | Meaning | Default |
|---|---|---|
| `-o`, `--out-dir <DIR>` | Write outputs into `DIR` (created if missing) | Next to the input |
| `-y`, `--overwrite` | Overwrite existing outputs | Off (skip them) |

### Video

| Option | Meaning | Default |
|---|---|---|
| `-c`, `--crf <N>` | Quality, lower = better. H.264: 0–51. WebM/VP9: 0–63 | 20 (H.264), 31 (VP9) |
| `-p`, `--preset <NAME>` | Speed vs. size: `ultrafast` … `veryslow` | `medium` |
| `-e`, `--encode` | Always re-encode video, never copy | Off |
| `-w`, `--width <PX>` | Resize to this width, keeping the aspect ratio (forces a re-encode) | Original size (GIF: 480) |
| `--fps <N>` | Change the frame rate (forces a re-encode) | Original (GIF: 12) |

### Trimming

| Option | Meaning | Default |
|---|---|---|
| `-s`, `--start <TIME>` | Start converting at `TIME` | Beginning |
| `-d`, `--duration <TIME>` | Only convert this much | Until the end |

### Audio

| Option | Meaning | Default |
|---|---|---|
| `-a`, `--no-audio` | Drop the audio track (not allowed with audio formats) | Keep audio |
| `-b`, `--audio-bitrate <RATE>` | Bitrate for AAC/MP3/Opus/Vorbis encoding, e.g. `128k`, `320k` | `192k` |

### Other

| Option / variable | Meaning |
|---|---|
| `-h`, `--help` | Show the built-in help |
| `FFMPEG_DIR` (environment variable) | Folder containing `ffmpeg.exe` and `ffprobe.exe`, if they aren't on `PATH` |

---

## 7. Quality and speed settings

These only matter when video is **re-encoded**. They have no effect on copied streams.

### CRF (quality)

For H.264 output (mp4, mov, mkv, avi):

| CRF | Result |
|---|---|
| `0` | Lossless, with enormous files |
| `17`–`18` | Visually indistinguishable from the source; good for screen recordings with small text |
| `20` | **Default.** Excellent quality at a sensible size |
| `23` | ffmpeg's own default; good for sharing |
| `26`–`28` | Smaller files with visible softening |

Each step of about ±6 roughly halves or doubles the file size.

For WebM (VP9) the scale is 0–63. The default is `31`, about the same as H.264 at 23. Use `24`–`28` for higher quality.

### Preset (speed)

From fastest to slowest: `ultrafast`, `superfast`, `veryfast`, `faster`, `fast`, `medium`, `slow`, `slower`, `veryslow`.

Slower presets give **smaller files at the same quality**. They don't make the video look better at a given CRF.
For WebM the preset is mapped to VP9's `-cpu-used` speed setting, from fast (`ultrafast` → 5) to slow (`veryslow` → 0).

---

## 8. What happens during a conversion

For each file, avi2mp4:

1. Runs `ffprobe` to read the codec of the first video and first audio stream, and the duration.
   Cover art embedded in audio files is not treated as video.
2. Decides **copy** or **encode** for each stream, using the tables in [section 4](#4-supported-formats).
   Video is always re-encoded if you ask for `-e`, `-w` or `--fps`, or if the output is a GIF.
3. Runs `ffmpeg` with these settings:
   - **H.264:** `libx264` with your CRF and preset, `yuv420p` colour format, and width and height rounded to even
     numbers (required for compatibility).
   - **VP9:** `libvpx-vp9` in constant-quality mode, with multi-threading enabled.
   - **GIF:** generates a colour palette from the clip first, then applies it. This gives far better colours than
     plain GIF conversion.
   - **HEVC copied into MP4/MOV:** tagged `hvc1` so it plays in Apple players.
   - **MP4/MOV/M4A:** `+faststart`, so playback starts immediately when streamed.
4. Shows progress as a percentage of the part being converted.
5. If ffmpeg fails, deletes the half-written output and prints ffmpeg's error, then continues with the next file.

At the end it prints how many files were converted, skipped and failed. The exit code is `0` if nothing failed and
`1` otherwise, which is useful in scripts.

Only the first video and first audio track are kept. Subtitles, chapters and extra audio tracks are dropped.

---

## 9. How long it takes

This depends almost entirely on whether streams are **copied** or **re-encoded**. The tool tells you which at the
start of each file.

- **Copy** (e.g. any → MKV, H.264 AVI → MP4, WAV → WAV): limited by disk speed, usually seconds.
- **Audio-only output:** fast, typically 50–100× real-time.
- **H.264 re-encode:** CPU-bound. As a rough guide, at `medium` a typical PC handles 720p at a few times
  real-time and 1080p at around real-time. `-p veryfast` is about 2–3× quicker.
- **VP9 (WebM) re-encode:** noticeably slower than H.264, often below real-time. Use `-p fast` or quicker presets
  for big batches.
- **GIF:** depends on the length and width, but clips are usually short, so it's quick.

**Real example:** `terraforn.avi` (4 min 24 s, 1280×720, UtVideo lossless video + PCM audio) went to MP4 at
**2,172 MB → 19.4 MB**. Lossless recordings like this always need re-encoding for MP4 and shrink dramatically.

---

## 10. Troubleshooting

**`error: `ffmpeg` not found`**
ffmpeg isn't on your `PATH`. Close and reopen PowerShell after installing it. If that doesn't help, point the
tool at it directly:
```powershell
$env:FFMPEG_DIR = "C:\Users\MasterMind47\AppData\Local\Microsoft\WinGet\Packages\Gyan.FFmpeg_Microsoft.Winget.Source_8wekyb3d8bbwe\ffmpeg-9.0.2-full_build\bin"
```
To find the folder on your machine, run `(Get-Command ffmpeg).Source` in a working terminal. The folder name changes
when ffmpeg is updated.

**`cargo : The term 'cargo' is not recognized`**
Reopen PowerShell, or run `& "$env:USERPROFILE\.cargo\bin\cargo.exe" build --release`.

**winget fails with `0x8a15005e ... server certificate did not match`**
Add `--source winget` to the install command (see 1.1).

**`Unknown encoder 'libvpx-vp9'` / `'libopus'` / `'libmp3lame'`**
Your ffmpeg build lacks that encoder. Install the full build: `winget install --id Gyan.FFmpeg -e --source winget`.

**`skipped (output exists, use -y to overwrite)`**
An output with that name already exists. Add `-y` to replace it, or `-o <folder>` to write elsewhere.

**`error: no matching media files found`**
The folder has no files with a supported extension (other than the target format). Check `--from`, or add `-r` if
the files are in subfolders.

**`no video stream to make a GIF from`** or **`no audio stream found`**
The input doesn't contain the kind of stream that format needs. For example, you can't make a GIF from an MP3.

**`unknown format` / `unknown preset` / `--crf must be ...`**
Check the spelling against [section 6](#6-all-options). `--crf` above 51 is only allowed for WebM.

**The trimmed clip is longer than asked or starts early**
The video was copied, so the cut snapped to a keyframe. Add `-e` for an exact cut.

**The progress line never appears**
ffprobe couldn't read the duration (common with damaged files). The conversion still runs.

**A copied video plays with glitches or wrong timing**
Some files store their streams oddly. Force a clean re-encode with `-e -y`.

**The output looks blurry**
Use a lower CRF, e.g. `-c 17 -y` (or `-c 24` for WebM).

---

## 11. Project layout

```
avi2mp4/
├── Cargo.toml      Project definition (no external crates)
├── README.md       This guide
└── src/
    └── main.rs     The whole program
```

Useful places in [src/main.rs](src/main.rs) if you want to change behaviour:

| To change | Edit |
|---|---|
| Which codecs are copied for each format | `Format::copies_video` / `Format::copies_audio` |
| Audio encoders and their settings | `Format::audio_encoder` |
| Default CRF, preset, bitrate | `parse_args()` and `convert()` |
| GIF defaults (12 fps, 480 px) | The `Format::Gif` branch in `convert()` |
| Extensions picked up from folders | `INPUT_EXTS` |
| Add a new output format | Add it to the `Format` enum and `FORMATS`, then update `is_audio`, `copies_video`, `copies_audio`, `video_encoder`, `audio_encoder`, and the video branch of `convert()` |

After editing, run `cargo build --release` again (and `cargo install --path .` if you installed it).

---

## 12. Uninstalling

```powershell
cargo uninstall avi2mp4                 # only if you used `cargo install`
winget uninstall --id Gyan.FFmpeg
rustup self uninstall
```

Then delete the `avi2mp4` folder.
