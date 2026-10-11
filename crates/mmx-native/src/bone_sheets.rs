//! Decode private Bone Animation Editor (`bae-sheet` v1) exports for the core.
//! The ROM and sheet assets stay outside the compiled binary and source tree.

use mmx_core::bone_animation::{BoneClip, BoneFrame, BoneSheet};
use mmx_core::{Character, Game};
use serde_json::Value;
use std::path::{Path, PathBuf};

struct Page {
    width: usize,
    height: usize,
    rgba: Vec<u8>,
}

pub const DEFAULT_VILE_SPRITE: &str = "roms/derived/bone_sheets/user_sources/vile_idle.png";

pub fn vile_sprite_path(override_path: Option<&str>, rom_path: &Path) -> PathBuf {
    if let Some(path) = override_path { return PathBuf::from(path); }
    let relative = PathBuf::from(DEFAULT_VILE_SPRITE);
    if relative.is_file() { return relative; }
    if let Some(rom_dir) = rom_path.parent() {
        let beside_rom = rom_dir.join("derived/bone_sheets/user_sources/vile_idle.png");
        if beside_rom.is_file() { return beside_rom; }
    }
    if let Ok(binary) = std::env::current_exe() {
        for parent in binary.ancestors().skip(1).take(4) {
            let candidate = parent.join(DEFAULT_VILE_SPRITE);
            if candidate.is_file() { return candidate; }
        }
    }
    relative
}

fn field<'a>(value: &'a Value, key: &str) -> Result<&'a Value, String> {
    value.get(key).ok_or_else(|| format!("missing {key}"))
}

fn number(value: &Value, label: &str) -> Result<usize, String> {
    value.as_u64().and_then(|n| usize::try_from(n).ok())
        .ok_or_else(|| format!("bad {label}"))
}

fn pair_i16(value: &Value, label: &str) -> Result<[i16; 2], String> {
    let a = value.as_array().ok_or_else(|| format!("bad {label}"))?;
    if a.len() != 2 { return Err(format!("bad {label}")); }
    let convert = |value: &Value| -> Result<i16, String> {
        let n = value.as_f64().ok_or_else(|| format!("bad {label}"))?;
        if !n.is_finite() || n < f64::from(i16::MIN) || n > f64::from(i16::MAX) {
            return Err(format!("{label} outside i16 range"));
        }
        Ok(n.round() as i16)
    };
    Ok([convert(&a[0])?, convert(&a[1])?])
}

fn png_page(path: &Path) -> Result<Page, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| format!("{}: {e}", path.display()))?;
    let mut bytes = vec![0; reader.output_buffer_size().ok_or("PNG size overflow")?];
    let info = reader.next_frame(&mut bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    let width = info.width as usize;
    let height = info.height as usize;
    if width == 0 || height == 0 || width > 4096 || height > 4096 { return Err(format!("{}: PNG dimensions outside 1..4096", path.display())); }
    let source = &bytes[..info.buffer_size()];
    let mut rgba = Vec::with_capacity(width * height * 4);
    match info.color_type {
        png::ColorType::Rgba => rgba.extend_from_slice(source),
        png::ColorType::Rgb => for p in source.chunks_exact(3) { rgba.extend_from_slice(&[p[0], p[1], p[2], 255]); },
        png::ColorType::GrayscaleAlpha => for p in source.chunks_exact(2) { rgba.extend_from_slice(&[p[0], p[0], p[0], p[1]]); },
        png::ColorType::Grayscale => for &v in source { rgba.extend_from_slice(&[v, v, v, 255]); },
        png::ColorType::Indexed => return Err(format!("{}: indexed PNG was not expanded", path.display())),
    }
    Ok(Page { width, height, rgba })
}

fn page_path(sheet_dir: &Path, name: &str) -> Result<PathBuf, String> {
    let relative = Path::new(name);
    if relative.is_absolute() || relative.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err(format!("page must stay inside sheet folder: {name}"));
    }
    Ok(sheet_dir.join(relative))
}

pub fn load_sheet(path: &Path) -> Result<BoneSheet, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let json: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let render_scale = match json.get("render_scale") {
        Some(value) => u8::try_from(number(value, "render_scale")?).map_err(|_| "bad render_scale")?,
        None => 1,
    };
    let source_faces_right = match json.get("source_facing") {
        None => None,
        Some(value) => Some(match value.as_str() {
            Some("right") => true,
            Some("left") => false,
            _ => return Err("source_facing must be right or left".into()),
        }),
    };
    let sheet_dir = path.parent().ok_or("sheet has no parent folder")?;
    let page_values = field(&json, "pages")?.as_array().ok_or("pages must be an array")?;
    let mut pages = Vec::new();
    for page in page_values {
        let name = field(page, "file")?.as_str().ok_or("page.file must be a path")?;
        let decoded = png_page(&page_path(sheet_dir, name)?)?;
        if let Some(size) = page.get("size") {
            let size = size.as_array().ok_or("page.size must be [width,height]")?;
            if size.len() != 2 || number(&size[0], "page.size")? != decoded.width
                || number(&size[1], "page.size")? != decoded.height {
                return Err(format!("{name}: page.size does not match PNG dimensions"));
            }
        }
        pages.push(decoded);
    }
    let animations = field(&json, "animations")?.as_array().ok_or("animations must be an array")?;
    let mut clips = Vec::new();
    for animation in animations {
        let name = field(animation, "name")?.as_str().ok_or("animation.name must be text")?;
        let fps = field(animation, "fps")?.as_f64().ok_or("animation.fps must be a number")? as f32;
        let looped = match animation.get("looped") {
            Some(value) => value.as_bool().ok_or("animation.looped must be true or false")?,
            None => true,
        };
        let frame_values = field(animation, "frames")?.as_array().ok_or("animation.frames must be an array")?;
        let mut frames = Vec::new();
        for frame in frame_values {
            let page = number(field(frame, "page")?, "frame.page")?;
            let page = pages.get(page).ok_or_else(|| format!("{name}: page index outside pages"))?;
            let rect = field(frame, "rect")?.as_array().ok_or("frame.rect must be [x,y,w,h]")?;
            if rect.len() != 4 { return Err(format!("{name}: frame.rect must have four numbers")); }
            let [x, y, w, h] = [
                number(&rect[0], "frame.rect")?,
                number(&rect[1], "frame.rect")?,
                number(&rect[2], "frame.rect")?,
                number(&rect[3], "frame.rect")?,
            ];
            if w == 0 || h == 0 || x.checked_add(w).is_none_or(|end| end > page.width)
                || y.checked_add(h).is_none_or(|end| end > page.height) {
                return Err(format!("{name}: frame rectangle outside PNG page"));
            }
            let width = u16::try_from(w).map_err(|_| format!("{name}: frame too wide"))?;
            let height = u16::try_from(h).map_err(|_| format!("{name}: frame too tall"))?;
            // bae-sheet pivots are already relative to the cropped rect.
            let pivot = pair_i16(field(frame, "pivot")?, "frame.pivot")?;
            let mut rgba = Vec::with_capacity(w * h * 4);
            for row in y..y + h {
                let start = (row * page.width + x) * 4;
                rgba.extend_from_slice(&page.rgba[start..start + w * 4]);
            }
            frames.push(BoneFrame { width, height, pivot, rgba });
        }
        clips.push(BoneClip { name: name.to_string(), fps, looped, frames });
    }
    let sheet = BoneSheet { clips, render_scale, source_faces_right };
    sheet.validate()?;
    Ok(sheet)
}

fn saber_contacts(json: &Value, character: Character, sheet_sha256: &str) -> Result<(f32, Vec<Option<mmx_core::saber::HitBox>>), String> {
    if json.get("format").and_then(Value::as_str) != Some("mmxrs-saber-contacts-v1")
        || json.get("character").and_then(Value::as_str) != Some(character.id())
        || json.get("clip").and_then(Value::as_str) != Some("attack")
        || json.get("coordinate_origin").and_then(Value::as_str) != Some("foot_pivot")
    { return Err("invalid saber contact metadata format or coordinate space".into()); }
    if !json.get("sheet_sha256").and_then(Value::as_str).is_some_and(|s| s.eq_ignore_ascii_case(sheet_sha256)) {
        return Err("saber contacts do not match the loaded sheet hash".into());
    }
    let fps = field(json, "fps")?.as_f64().ok_or("bad saber contact fps")?;
    if !fps.is_finite() || fps <= 0.0 || fps > 240.0 { return Err("bad saber contact fps".into()); }
    let values = field(json, "frames")?.as_array().ok_or("saber contact frames must be an array")?;
    let mut boxes = Vec::with_capacity(values.len());
    for (index, value) in values.iter().enumerate() {
        if value.get("index").and_then(Value::as_u64) != Some(index as u64) {
            return Err("saber contact frames must be contiguous and ordered".into());
        }
        let bounds = field(value, "bounds")?;
        if bounds.is_null() { boxes.push(None); continue; }
        let a = bounds.as_array().filter(|a| a.len() == 4).ok_or("bad saber contact bounds")?;
        let mut b = [0_i16; 4];
        for (i, v) in a.iter().enumerate() {
            b[i] = i16::try_from(v.as_i64().ok_or("saber bounds must be integer game pixels")?)
                .map_err(|_| "saber bounds outside i16 range")?;
            if i == 1 || i == 3 { b[i] = b[i].checked_add(16).ok_or("saber bounds overflow after pivot conversion")?; }
        }
        boxes.push(Some(mmx_core::saber::HitBox::from_bounds(b).ok_or("invalid saber contact rectangle")?));
    }
    Ok((fps as f32, boxes))
}

pub fn install(game: &mut Game, root: &Path, characters: &[Character]) -> Result<usize, String> {
    if !root.is_dir() { return Err(format!("bone sheet folder missing: {}", root.display())); }
    let mut loaded = 0;
    let mut seen = Vec::new();
    for &character in characters {
        if seen.contains(&character) { continue; }
        seen.push(character);
        let id = character.id();
        let path = root.join(id).join(format!("{id}.sheet.json"));
        if !path.is_file() {
            if matches!(character, Character::Bass | Character::ProtoMan) {
                return Err(format!("{} needs a sheet at {}", character.label(), path.display()));
            }
            continue;
        }
        let sheet = load_sheet(&path)?;
        game.set_bone_sheet(character, sheet).map_err(|e| format!("{}: {e}", path.display()))?;
        let contacts = root.join(id).join(format!("{id}.saber_contacts.json"));
        if contacts.is_file() {
            use sha2::{Digest, Sha256};
            let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let sheet_hash = crate::headless::hex(&Sha256::digest(bytes));
            let metadata: Value = serde_json::from_str(&std::fs::read_to_string(&contacts).map_err(|e| format!("{}: {e}", contacts.display()))?)
                .map_err(|e| format!("{}: {e}", contacts.display()))?;
            let (fps, boxes) = saber_contacts(&metadata, character, &sheet_hash)
                .map_err(|e| format!("{}: {e}", contacts.display()))?;
            game.set_saber_contact_boxes(character, fps, boxes)
                .map_err(|e| format!("{}: {e}", contacts.display()))?;
        }
        loaded += 1;
    }
    if loaded == 0 { return Err(format!("no selected character sheets found in {}", root.display())); }
    Ok(loaded)
}

/// Vile's user-supplied on-foot pose, independent of the optional animated
/// pack. Without this crop the X1 ROM's `$50` sprite has the wrong silhouette.
pub fn install_static_vile(game: &mut Game, path: &Path) -> Result<(), String> {
    let page = png_page(path)?;
    let width = u16::try_from(page.width).map_err(|_| "Vile sprite too wide")?;
    let height = u16::try_from(page.height).map_err(|_| "Vile sprite too tall")?;
    game.set_static_sprite(Character::Vile, BoneFrame {
        width,
        height,
        pivot: [16, 44],
        rgba: page.rgba,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saber_contacts_convert_foot_pivot_and_reject_stale_or_invalid_metadata() {
        let mut json = serde_json::json!({
            "format":"mmxrs-saber-contacts-v1", "character":"zero", "clip":"attack",
            "coordinate_origin":"foot_pivot", "sheet_sha256":"abcd", "fps":60,
            "frames":[{"index":0,"bounds":[5,-20,29,-2]},{"index":1,"bounds":null}]
        });
        let (fps, boxes) = saber_contacts(&json, Character::Zero, "abcd").unwrap();
        assert_eq!(fps, 60.0);
        assert_eq!(boxes, vec![mmx_core::saber::HitBox::from_bounds([5,-4,29,14]), None]);
        assert!(saber_contacts(&json, Character::Zero, "stale").is_err());
        assert!(saber_contacts(&json, Character::Sigma, "abcd").is_err());
        json["frames"][1]["index"] = serde_json::json!(0);
        assert!(saber_contacts(&json, Character::Zero, "abcd").is_err());
        json["frames"][1]["index"] = serde_json::json!(1);
        json["frames"][0]["bounds"] = serde_json::json!([0,32760,10,32767]);
        assert!(saber_contacts(&json, Character::Zero, "abcd").is_err());
    }

    #[test]
    fn decodes_cropped_rgba_and_local_pivot() {
        let dir = std::env::temp_dir().join(format!("mmx-bone-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let png_path = dir.join("page.png");
        let file = std::fs::File::create(&png_path).unwrap();
        let mut encoder = png::Encoder::new(file, 2, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&[1,2,3,255, 4,5,6,128]).unwrap();
        let json_path = dir.join("test.sheet.json");
        std::fs::write(&json_path, r#"{
            "pages":[{"file":"page.png","size":[2,1]}],
            "animations":[{"name":"idle","fps":60,"frames":[{"page":0,"rect":[1,0,1,1],"pivot":[-1,3]}]}]
        }"#).unwrap();
        let sheet = load_sheet(&json_path).unwrap();
        assert_eq!(sheet.clips[0].frames[0].rgba, [4,5,6,128]);
        assert_eq!(sheet.clips[0].frames[0].pivot, [-1,3]);
        assert_eq!(sheet.render_scale, 1);
        assert_eq!(sheet.source_faces_right, None);
        assert!(sheet.clips[0].looped);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn high_resolution_sheet_keeps_pixels_pivots_direction_and_one_shot_timing() {
        let dir = std::env::temp_dir().join(format!("mmx-hd-sheet-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pixels: Vec<u8> = (0..16).flat_map(|n| [n, 10, 20, 255]).collect();
        let file = std::fs::File::create(dir.join("page.png")).unwrap();
        let mut encoder = png::Encoder::new(file, 4, 4);
        encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&pixels).unwrap();
        let mut json = serde_json::json!({
            "format":"bae-sheet", "version":1, "render_scale":4, "source_facing":"right",
            "pages":[{"file":"page.png", "size":[4,4]}],
            "animations":[{"name":"idle", "fps":60, "looped":false,
                "frames":[{"page":0,"rect":[0,0,4,4],"pivot":[2,4]}]}]
        });
        let path = dir.join("test.sheet.json");
        std::fs::write(&path, json.to_string()).unwrap();
        let sheet = load_sheet(&path).unwrap();
        assert_eq!(sheet.render_scale, 4);
        assert_eq!(sheet.source_faces_right, Some(true));
        assert_eq!(sheet.clips[0].fps, 60.0);
        assert!(!sheet.clips[0].looped);
        assert_eq!(sheet.clips[0].frames[0].pivot, [2,4]);
        assert_eq!(sheet.clips[0].frames[0].rgba, pixels, "loader must not reduce source detail");
        json["render_scale"] = serde_json::json!(3);
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(load_sheet(&path).unwrap_err().contains("render_scale"));
        json["render_scale"] = serde_json::json!(4);
        json["source_facing"] = serde_json::json!("up");
        std::fs::write(&path, json.to_string()).unwrap();
        assert!(load_sheet(&path).unwrap_err().contains("source_facing"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
