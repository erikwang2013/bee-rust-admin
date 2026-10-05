// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::fs;
use std::process::Command;

use crate::CliResult;

const OUT_DIR: &str = ".rusty";

#[derive(Clone, Copy, PartialEq)]
enum Mood {
    Cheerful,
    Debugging,
    Loaded,
    Dozing,
    Frazzled,
}

impl Mood {
    fn face(self) -> &'static str {
        match self {
            Mood::Cheerful => "(^o^)",
            Mood::Debugging => "o(°▽°)o",
            Mood::Loaded => "(≧▽≦)",
            Mood::Dozing => "(-_-)zZ",
            Mood::Frazzled => "(#°益°)",
        }
    }

    fn msg(self) -> &'static str {
        match self {
            Mood::Cheerful => "测试全绿，绒毛炸起，翅膀高频振动",
            Mood::Debugging => "悬停在半空追一个难缠的 bug，翅膀只剩残影",
            Mood::Loaded => "工具盒鼓鼓的，新驱动接入成功，飞得歪歪扭扭",
            Mood::Dozing => "深夜没有流量，蹲在蜂巢门口打瞌睡，触角耷拉",
            Mood::Frazzled => "尾针竖起来了——项目里可能躺着密钥文件！",
        }
    }
}

/// Show the project pet bee: detect project state, render the matching
/// expression as an SVG image, and point at it.
pub fn pet() -> CliResult {
    let mood = detect_mood()?;
    let svg = render_svg(mood);

    fs::create_dir_all(OUT_DIR).map_err(|e| e.to_string())?;
    let path = format!("{OUT_DIR}/current.svg");
    fs::write(&path, svg).map_err(|e| e.to_string())?;

    println!("Rusty 当前状态：{} {}", mood.face(), mood.msg());
    println!("当前形象已渲染：{path}（浏览器打开即可查看）");
    Ok(())
}

/// Map project state to a mood. Frazzled (secrets in the tree) wins, then
/// uncommitted work, then night time, then a driver/crate commit.
fn detect_mood() -> Result<Mood, String> {
    let status = git(&["status", "--porcelain"])?;
    if contains_secret(&status) {
        return Ok(Mood::Frazzled);
    }
    if !status.trim().is_empty() {
        return Ok(Mood::Debugging);
    }
    if is_night() {
        return Ok(Mood::Dozing);
    }
    if recent_commit_touched_drivers()? {
        return Ok(Mood::Loaded);
    }
    Ok(Mood::Cheerful)
}

fn contains_secret(porcelain: &str) -> bool {
    let patterns = [".env", ".pem", "id_rsa", "secret", "credential", "token"];
    porcelain.lines().any(|line| {
        let name = line.trim_start_matches(' ').split_once(' ').map(|x| x.1).unwrap_or(line);
        patterns.iter().any(|p| name.contains(p))
    })
}

fn is_night() -> bool {
    let hour = Command::new("date")
        .arg("+%H")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u8>().ok())
        .unwrap_or(12);
    // 夜间：23:00–05:59
    !(6..23).contains(&hour)
}

fn recent_commit_touched_drivers() -> Result<bool, String> {
    let stat = git(&["show", "--name-status", "--format=", "HEAD"])?;
    Ok(stat.lines().any(|l| {
        l.starts_with("A\tcrates/") || l.starts_with("M\tcrates/") || l.contains("Cargo.toml")
    }))
}

fn git(args: &[&str]) -> Result<String, String> {
    let out = Command::new("git").args(args).output().map_err(|e| e.to_string())?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

const BASE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" width="512" height="512">
  <defs>
    <radialGradient id="bellyGrad" cx="45%" cy="32%" r="72%">
      <stop offset="0%" stop-color="#FFC62E"/>
      <stop offset="70%" stop-color="#F5B301"/>
      <stop offset="100%" stop-color="#DE9E00"/>
    </radialGradient>
    <radialGradient id="headGrad" cx="42%" cy="30%" r="75%">
      <stop offset="0%" stop-color="#FFC62E"/>
      <stop offset="100%" stop-color="#F0AF00"/>
    </radialGradient>
    <linearGradient id="wingGrad" x1="0" y1="0" x2="1" y2="1">
      <stop offset="0%" stop-color="#EAF5FC"/>
      <stop offset="100%" stop-color="#CFE8F5"/>
    </linearGradient>
    <clipPath id="bellyClip">
      <ellipse cx="256" cy="332" rx="118" ry="128"/>
    </clipPath>
  </defs>

  <g fill="url(#wingGrad)" stroke="#9FC9E8" stroke-width="2" opacity="0.6">
    <ellipse cx="110" cy="150" rx="85" ry="105" transform="rotate(-22 110 150)"/>
    <ellipse cx="402" cy="150" rx="85" ry="105" transform="rotate(22 402 150)"/>
    <ellipse cx="158" cy="238" rx="60" ry="80" transform="rotate(-42 158 238)"/>
    <ellipse cx="354" cy="238" rx="60" ry="80" transform="rotate(42 354 238)"/>
  </g>

  <g stroke="#1F1A17" stroke-width="7" stroke-linecap="round" fill="none">
    <path d="M198 452 Q180 482 174 502"/>
    <path d="M314 452 Q332 482 338 502"/>
  </g>

  <ellipse cx="256" cy="332" rx="118" ry="128" fill="url(#bellyGrad)" stroke="#C98A00" stroke-width="3"/>
  <path d="M152 290 Q256 208 360 290" fill="none" stroke="#FFE38A" stroke-width="7" stroke-linecap="round" opacity="0.75"/>
  <g clip-path="url(#bellyClip)">
    <path d="M128 378 Q256 350 384 378 L384 408 Q256 380 128 408 Z" fill="#1F1A17"/>
    <path d="M124 418 Q256 390 388 418 L388 448 Q256 420 124 448 Z" fill="#1F1A17"/>
    <path d="M120 458 Q256 430 392 458 L392 486 Q256 460 120 486 Z" fill="#1F1A17"/>
  </g>

  <ellipse cx="256" cy="238" rx="46" ry="30" fill="url(#headGrad)" stroke="#C98A00" stroke-width="3"/>
  <circle cx="256" cy="178" r="84" fill="url(#headGrad)" stroke="#C98A00" stroke-width="3"/>

  {ANTENNAE}

  {EYES}

  {MOUTH}

  <ellipse cx="178" cy="196" rx="15" ry="9" fill="#E8890C" opacity="0.4"/>
  <ellipse cx="334" cy="196" rx="15" ry="9" fill="#E8890C" opacity="0.4"/>

  <g>
    <rect x="234" y="248" width="44" height="30" rx="7" fill="#B7410E" stroke="#8A320A" stroke-width="3"/>
    <path d="M234 260 L278 260" stroke="#8A320A" stroke-width="3"/>
    <rect x="252" y="252" width="8" height="6" rx="2" fill="#E8890C"/>
    <path d="M170 264 Q205 284 240 268" fill="none" stroke="#1F1A17" stroke-width="7" stroke-linecap="round"/>
    <path d="M342 264 Q307 284 272 268" fill="none" stroke="#1F1A17" stroke-width="7" stroke-linecap="round"/>
  </g>

  {EXTRA}

  <text x="256" y="500" text-anchor="middle" font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="30" letter-spacing="7" fill="#1F1A17">bee-rust</text>
</svg>
"##;

/// The four mood-visible SVG regions (antennae, eyes, mouth, extra props).
/// Keep each mood atomic so the pieces can't drift out of sync.
fn parts(mood: Mood) -> (&'static str, &'static str, &'static str, &'static str) {
    const UP: &str = r##"<g stroke="#1F1A17" stroke-width="6" stroke-linecap="round" fill="#1F1A17">
    <path d="M224 100 Q208 72 212 56" fill="none"/><circle cx="212" cy="52" r="7"/>
    <path d="M288 100 Q304 72 300 56" fill="none"/><circle cx="300" cy="52" r="7"/>
  </g>"##;
    const EYES_UP: &str = r##"<ellipse cx="205" cy="162" rx="30" ry="38" fill="#2A1E16"/>
  <ellipse cx="307" cy="162" rx="30" ry="38" fill="#2A1E16"/>
  <circle cx="193" cy="146" r="9" fill="#FFFFFF"/><circle cx="295" cy="146" r="9" fill="#FFFFFF"/>"##;
    const MOUTH_UP: &str = r##"<path d="M244 216 Q256 228 268 216" fill="none" stroke="#1F1A17" stroke-width="5" stroke-linecap="round"/>"##;

    let debug_eyes = r##"<circle cx="205" cy="165" r="15" fill="#2A1E16"/>
  <circle cx="307" cy="165" r="15" fill="#2A1E16"/>
  <circle cx="201" cy="160" r="4" fill="#FFFFFF"/><circle cx="303" cy="160" r="4" fill="#FFFFFF"/>"##;
    let debug_mouth = r##"<ellipse cx="256" cy="220" rx="9" ry="11" fill="#1F1A17"/>"##;
    let debug_extra = r##"<path d="M196 130 Q200 118 212 114" fill="none" stroke="#9FC9E8" stroke-width="6" stroke-linecap="round"/>"##;

    let loaded_eyes = r##"<path d="M178 168 Q205 150 232 168" fill="none" stroke="#1F1A17" stroke-width="7" stroke-linecap="round"/>
  <path d="M280 168 Q307 150 334 168" fill="none" stroke="#1F1A17" stroke-width="7" stroke-linecap="round"/>"##;
    let loaded_mouth = r##"<path d="M236 216 Q256 236 276 216" fill="none" stroke="#1F1A17" stroke-width="6" stroke-linecap="round"/>"##;
    let loaded_extra = r##"<circle cx="212" cy="232" r="7" fill="#FFC62E"/><circle cx="300" cy="232" r="7" fill="#FFC62E"/><circle cx="256" cy="224" r="5" fill="#FFC62E"/>"##;

    let dozing_antennae = r##"<g stroke="#1F1A17" stroke-width="6" stroke-linecap="round" fill="#1F1A17">
    <path d="M224 100 Q208 124 192 118" fill="none"/>
    <path d="M288 100 Q304 124 320 118" fill="none"/>
  </g>"##;
    let dozing_eyes = r##"<path d="M182 165 L228 165" stroke="#1F1A17" stroke-width="7" stroke-linecap="round"/>
  <path d="M284 165 L330 165" stroke="#1F1A17" stroke-width="7" stroke-linecap="round"/>"##;
    let dozing_mouth = r##"<path d="M248 224 Q256 227 264 224" fill="none" stroke="#1F1A17" stroke-width="4" stroke-linecap="round"/>"##;
    let dozing_extra =
        r##"<text x="330" y="70" font-family="monospace" font-size="30" fill="#9FC9E8">zZ</text>"##;

    let frazzled_eyes = r##"<path d="M178 130 L228 144" stroke="#1F1A17" stroke-width="8" stroke-linecap="round"/>
  <path d="M334 130 L284 144" stroke="#1F1A17" stroke-width="8" stroke-linecap="round"/>
  <ellipse cx="205" cy="168" rx="26" ry="30" fill="#2A1E16"/>
  <ellipse cx="307" cy="168" rx="26" ry="30" fill="#2A1E16"/>
  <circle cx="196" cy="156" r="6" fill="#FFFFFF"/><circle cx="298" cy="156" r="6" fill="#FFFFFF"/>"##;
    let frazzled_mouth = r##"<path d="M248 220 L264 220" stroke="#1F1A17" stroke-width="6" stroke-linecap="round"/>"##;
    let frazzled_extra = r##"<path d="M344 434 L376 458 L346 462 Z" fill="#1F1A17"/>"##;

    match mood {
        Mood::Cheerful => (UP, EYES_UP, MOUTH_UP, ""),
        Mood::Debugging => (UP, debug_eyes, debug_mouth, debug_extra),
        Mood::Loaded => (UP, loaded_eyes, loaded_mouth, loaded_extra),
        Mood::Dozing => (dozing_antennae, dozing_eyes, dozing_mouth, dozing_extra),
        Mood::Frazzled => (UP, frazzled_eyes, frazzled_mouth, frazzled_extra),
    }
}

fn render_svg(mood: Mood) -> String {
    let (a, e, m, x) = parts(mood);
    BASE.replace("{ANTENNAE}", a).replace("{EYES}", e).replace("{MOUTH}", m).replace("{EXTRA}", x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_detection_flags_credentials() {
        assert!(contains_secret("?? .env.local\n M src/main.rs"));
        assert!(contains_secret("?? keys/id_rsa"));
        assert!(!contains_secret(" M src/lib.rs\n?? docs/rusty.svg"));
    }

    #[test]
    fn every_mood_renders_a_distinct_svg() {
        for mood in [Mood::Cheerful, Mood::Debugging, Mood::Loaded, Mood::Dozing, Mood::Frazzled] {
            let svg = render_svg(mood);
            assert!(svg.starts_with("<svg") && svg.ends_with("</svg>\n"));
            assert_eq!(svg.matches("<!--").count(), 0, "no comment leftovers");
        }
        assert_ne!(render_svg(Mood::Cheerful), render_svg(Mood::Frazzled));
    }
}
