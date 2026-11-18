//! 通过系统 ffmpeg 转换音频文件。
//!
//! 依赖 PATH 中的 `ffmpeg`（或用 `FOOBAR_RS_FFMPEG` 指定）。同名输出不覆盖，
//! 冲突时追加 ` (1)`、` (2)`。转换结果不会自动导入媒体库，需要重新扫描。

use std::{
    env,
    ffi::OsStr,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConvertFormat {
    Flac,
    Mp3,
    Opus,
    Wav,
}

impl ConvertFormat {
    pub fn parse(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "mp3" => Self::Mp3,
            "opus" => Self::Opus,
            "wav" => Self::Wav,
            _ => Self::Flac,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Flac => "flac",
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
            Self::Wav => "wav",
        }
    }

    pub fn codec_args(self) -> &'static [&'static str] {
        match self {
            Self::Flac => &["-c:a", "flac", "-compression_level", "5"],
            Self::Mp3 => &["-c:a", "libmp3lame", "-q:a", "2"],
            Self::Opus => &["-c:a", "libopus", "-b:a", "160k"],
            Self::Wav => &["-c:a", "pcm_s16le"],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BatchReport {
    pub converted: usize,
    pub skipped: usize,
    pub failed: usize,
    pub errors: Vec<String>,
}

pub fn ffmpeg_path() -> Option<PathBuf> {
    find_ffmpeg(
        env::var_os("FOOBAR_RS_FFMPEG").as_deref(),
        env::var_os("PATH").as_deref(),
    )
}

/// 可注入环境变量的探测入口，便于测试“没有 ffmpeg”路径。
fn find_ffmpeg(explicit: Option<&OsStr>, path_var: Option<&OsStr>) -> Option<PathBuf> {
    if let Some(explicit) = explicit.filter(|value| !value.is_empty()) {
        let path = PathBuf::from(explicit);
        return probe(&path).then_some(path);
    }
    let path_var = path_var?;
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    for dir in env::split_paths(path_var) {
        let candidate = dir.join(name);
        if probe(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn probe(path: &Path) -> bool {
    Command::new(path)
        .arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// 目标已存在时追加 ` (1)`, ` (2)`…，同目录同格式也不会覆盖源文件。
pub fn output_path(input: &Path, out_dir: &Path, format: ConvertFormat) -> PathBuf {
    let stem = input
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "track".to_owned());
    let extension = format.extension();
    let mut candidate = out_dir.join(format!("{stem}.{extension}"));
    let mut index = 1;
    while candidate.exists() {
        candidate = out_dir.join(format!("{stem} ({index}).{extension}"));
        index += 1;
    }
    candidate
}

pub fn convert_file(
    ffmpeg: &Path,
    input: &Path,
    out: &Path,
    format: ConvertFormat,
) -> Result<(), String> {
    let output = Command::new(ffmpeg)
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(input)
        .args(format.codec_args())
        .args(["-vn", "-map_metadata", "0", "-n"])
        .arg(out)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| format!("无法启动 ffmpeg：{err}"))?;

    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut tail: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .rev()
        .take(3)
        .collect();
    tail.reverse();
    Err(if tail.is_empty() {
        format!("ffmpeg 退出码 {}", output.status.code().unwrap_or(-1))
    } else {
        tail.join(" | ")
    })
}

pub fn convert_batch(inputs: &[PathBuf], out_dir: &Path, format: ConvertFormat) -> BatchReport {
    let Some(ffmpeg) = ffmpeg_path() else {
        return BatchReport {
            converted: 0,
            skipped: 0,
            failed: inputs.len(),
            errors: vec!["未找到 ffmpeg（可设 FOOBAR_RS_FFMPEG）".into()],
        };
    };
    convert_batch_with_ffmpeg(&ffmpeg, inputs, out_dir, format)
}

pub fn convert_batch_with_ffmpeg(
    ffmpeg: &Path,
    inputs: &[PathBuf],
    out_dir: &Path,
    format: ConvertFormat,
) -> BatchReport {
    let mut report = BatchReport::default();
    for input in inputs {
        if !input.is_file() {
            report.skipped += 1;
            continue;
        }
        let out = output_path(input, out_dir, format);
        match convert_file(ffmpeg, input, &out, format) {
            Ok(()) => report.converted += 1,
            Err(message) => {
                report.failed += 1;
                report
                    .errors
                    .push(format!("{}：{message}", input.display()));
            }
        }
    }
    report
}

pub fn report_status(report: &BatchReport) -> String {
    let mut text = format!(
        "已转换 {}，失败 {}，跳过 {}",
        report.converted, report.failed, report.skipped
    );
    if let Some(error) = report.errors.first() {
        text.push_str(&format!("；错误：{error}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn wav_bytes() -> Vec<u8> {
        let sample_rate = 8_000u32;
        let samples = sample_rate / 10;
        let data_len = samples * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        for i in 0..samples {
            wav.extend_from_slice(&((i % 64) as i16).to_le_bytes());
        }
        wav
    }

    #[test]
    fn parses_formats_and_arguments() {
        assert_eq!(ConvertFormat::parse("flac"), ConvertFormat::Flac);
        assert_eq!(ConvertFormat::parse("MP3"), ConvertFormat::Mp3);
        assert_eq!(ConvertFormat::parse("opus"), ConvertFormat::Opus);
        assert_eq!(ConvertFormat::parse("wav"), ConvertFormat::Wav);
        assert_eq!(ConvertFormat::parse("aac"), ConvertFormat::Flac);
        assert_eq!(ConvertFormat::Mp3.extension(), "mp3");
        assert_eq!(
            ConvertFormat::Flac.codec_args(),
            ["-c:a", "flac", "-compression_level", "5"]
        );
        assert_eq!(
            ConvertFormat::Mp3.codec_args(),
            ["-c:a", "libmp3lame", "-q:a", "2"]
        );
        assert_eq!(
            ConvertFormat::Opus.codec_args(),
            ["-c:a", "libopus", "-b:a", "160k"]
        );
        assert_eq!(ConvertFormat::Wav.codec_args(), ["-c:a", "pcm_s16le"]);
    }

    #[test]
    fn output_path_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();
        let input = dir.path().join("song.flac");
        fs::write(&input, b"x").unwrap();

        let first = output_path(&input, &out, ConvertFormat::Mp3);
        assert_eq!(first, out.join("song.mp3"));
        fs::write(&first, b"1").unwrap();
        let second = output_path(&input, &out, ConvertFormat::Mp3);
        assert_eq!(second, out.join("song (1).mp3"));
        fs::write(&second, b"2").unwrap();
        assert_eq!(
            output_path(&input, &out, ConvertFormat::Mp3),
            out.join("song (2).mp3")
        );

        // 同目录同格式时源文件本身已存在，必须换名。
        assert_eq!(
            output_path(&input, dir.path(), ConvertFormat::Flac),
            dir.path().join("song (1).flac")
        );
    }

    #[test]
    fn missing_ffmpeg_is_detected() {
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(find_ffmpeg(None, Some(empty.path().as_os_str())), None);
    }

    #[test]
    fn report_status_includes_counts_and_first_error() {
        let report = BatchReport {
            converted: 2,
            skipped: 1,
            failed: 1,
            errors: vec!["bad file".into()],
        };
        assert_eq!(
            report_status(&report),
            "已转换 2，失败 1，跳过 1；错误：bad file"
        );
    }

    #[cfg(unix)]
    #[test]
    fn batch_counts_success_skip_and_failure() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("song.wav");
        fs::write(&input, wav_bytes()).unwrap();
        let missing = dir.path().join("missing.wav");
        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();

        let ok = convert_batch_with_ffmpeg(
            Path::new("/bin/true"),
            &[input.clone(), missing.clone()],
            &out,
            ConvertFormat::Flac,
        );
        assert_eq!(ok.converted, 1);
        assert_eq!(ok.skipped, 1);
        assert_eq!(ok.failed, 0);

        let failed = convert_batch_with_ffmpeg(
            Path::new("/bin/false"),
            &[input, missing],
            &out,
            ConvertFormat::Flac,
        );
        assert_eq!(failed.converted, 0);
        assert_eq!(failed.skipped, 1);
        assert_eq!(failed.failed, 1);
        assert_eq!(failed.errors.len(), 1);
    }

    #[test]
    fn real_ffmpeg_converts_short_wav_when_available() {
        let Some(ffmpeg) = ffmpeg_path() else {
            eprintln!("跳过真实转换：未找到 ffmpeg");
            return;
        };
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("tone.wav");
        fs::write(&input, wav_bytes()).unwrap();
        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();

        let flac = output_path(&input, &out, ConvertFormat::Flac);
        convert_file(&ffmpeg, &input, &flac, ConvertFormat::Flac).unwrap();
        let flac_bytes = fs::read(&flac).unwrap();
        assert!(flac_bytes.len() > 4);
        assert_eq!(&flac_bytes[..4], b"fLaC");

        let wav = output_path(&input, &out, ConvertFormat::Wav);
        convert_file(&ffmpeg, &input, &wav, ConvertFormat::Wav).unwrap();
        let wav_bytes = fs::read(&wav).unwrap();
        assert!(wav_bytes.len() > 4);
        assert_eq!(&wav_bytes[..4], b"RIFF");
    }
}
