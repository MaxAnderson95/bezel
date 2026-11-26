//! Every fixture scene (`bezel --list-scenes`), rendered headless in light and dark, compared
//! with its golden in `tests/golden`. After an intended visual change, rewrite them with
//! `BEZEL_UPDATE_GOLDENS=1 cargo test -p bezel --test goldens` and look at every changed image.
//! A render that misses its golden is written to `target/tmp/golden-actual`.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::process::Command;

/// The window in points, at Retina scale, as most people see Bezel.
const SIZE: &str = "1024x640";
const SCALE: &str = "2";

/// GPU drivers differ in the last bits of blending and glyph coverage; a layout or colour
/// change moves far more pixels, by far more, than this (Hex's tolerance).
const CHANNEL_TOLERANCE: u8 = 12;
const DIFFERING_PIXELS_PER: usize = 500;

#[test]
fn every_scene_matches_its_golden() {
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let actual = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-actual");
    let home = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden-home");
    let _ = std::fs::remove_dir_all(&actual);
    std::fs::create_dir_all(&actual).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    let update = std::env::var_os("BEZEL_UPDATE_GOLDENS").is_some();
    if update {
        std::fs::create_dir_all(&goldens).unwrap();
    }
    let scenes = scenes(&home);
    let mut misses = Vec::new();
    for scene in &scenes {
        for theme in ["light", "dark"] {
            let name = format!("{scene}-{theme}.png");
            let render = actual.join(&name);
            snap(&home, scene, theme, &render);
            let golden = goldens.join(&name);
            if update {
                std::fs::copy(&render, &golden).unwrap();
                std::fs::remove_file(&render).unwrap();
                continue;
            }
            match compare(&render, &golden) {
                Ok(()) => std::fs::remove_file(&render).unwrap(),
                Err(miss) => misses.push(format!("{name}: {miss}")),
            }
        }
    }
    if update {
        // A golden for a scene that no longer exists would otherwise stay forever.
        for entry in std::fs::read_dir(&goldens).unwrap() {
            let path = entry.unwrap().path();
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
            let scene = stem.trim_end_matches("-light").trim_end_matches("-dark");
            if !scenes.iter().any(|s| s == scene) {
                std::fs::remove_file(path).unwrap();
            }
        }
    }
    assert!(
        misses.is_empty(),
        "{} of {} renders missed their goldens; the renders are in {}:\n{}",
        misses.len(),
        scenes.len() * 2,
        actual.display(),
        misses.join("\n")
    );
}

/// Bezel with its logs and data under `home`, never the person's.
fn bezel(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_bezel"));
    command
        .env("HOME", home)
        .env("XDG_STATE_HOME", home)
        .env("XDG_DATA_HOME", home);
    command
}

fn scenes(home: &Path) -> Vec<String> {
    let output = bezel(home).arg("--list-scenes").output().unwrap();
    assert!(output.status.success(), "--list-scenes failed: {output:?}");
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn snap(home: &Path, scene: &str, theme: &str, out: &Path) {
    let output = bezel(home)
        .args(["--snap".as_ref(), out.as_os_str()])
        .args([
            "--scene", scene, "--theme", theme, "--size", SIZE, "--scale", SCALE, "--settle",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "rendering {scene} in {theme} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn compare(render: &Path, golden: &Path) -> Result<(), String> {
    if !golden.exists() {
        return Err("no golden; run with BEZEL_UPDATE_GOLDENS=1".into());
    }
    let (width, height, pixels) = read_png(render);
    let (golden_width, golden_height, golden) = read_png(golden);
    if (width, height) != (golden_width, golden_height) {
        return Err(format!(
            "{width}×{height}, the golden is {golden_width}×{golden_height}"
        ));
    }
    let differing = pixels
        .chunks(4)
        .zip(golden.chunks(4))
        .filter(|(a, b)| a.iter().zip(b.iter()).any(|(x, y)| x.abs_diff(*y) > CHANNEL_TOLERANCE))
        .count();
    let limit = pixels.len() / 4 / DIFFERING_PIXELS_PER;
    if differing > limit {
        return Err(format!("{differing} pixels differ (limit {limit})"));
    }
    Ok(())
}

fn read_png(path: &Path) -> (u32, u32, Vec<u8>) {
    let decoder = png::Decoder::new(BufReader::new(File::open(path).unwrap()));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    pixels.truncate(info.buffer_size());
    (info.width, info.height, pixels)
}
