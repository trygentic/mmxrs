//! On-screen text drawn over the game picture: the F2 / `--show-fps` speed
//! readout and short notices such as "no audio device". 3x5 pixel font from
//! `vs_select`, on a dark box so it reads over any scenery.

/// `.`, `:` and `%` on top of the VS screen's letters and digits.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '.' => [0, 0, 0, 0, 2],
        ':' => [0, 2, 0, 2, 0],
        '%' => [5, 1, 2, 4, 5],
        c => crate::vs_select::glyph(c.to_ascii_uppercase()),
    }
}

/// Lines of text in the top-left corner. `scale` is the frame's pixels per
/// SNES pixel (`Game::frame_scale`); the font is drawn at 1 SNES pixel.
pub fn draw(rgba: &mut [u8], width: usize, height: usize, lines: &[&str], scale: usize) {
    let s = scale.max(1);
    let (pad, adv, line_h) = (2 * s, 4 * s, 7 * s);
    let w = lines.iter().map(|l| l.chars().count()).max().unwrap_or(0) * adv + 2 * pad - s;
    let h = lines.len() * line_h + 2 * pad - 2 * s;
    let (x0, y0) = (2 * s, 2 * s);
    fill(rgba, (width, height), (x0, y0, w, h), |p| {
        for c in &mut p[..3] {
            *c /= 4;
        }
    });
    for (row, line) in lines.iter().enumerate() {
        for (i, ch) in line.chars().enumerate() {
            for (dy, bits) in glyph(ch).into_iter().enumerate() {
                for dx in 0..3 {
                    if bits & (4 >> dx) != 0 {
                        let x = x0 + pad + i * adv + dx * s;
                        let y = y0 + pad + row * line_h + dy * s;
                        fill(rgba, (width, height), (x, y, s, s), |p| p[..3].copy_from_slice(&[0xF8, 0xF8, 0xF8]));
                    }
                }
            }
        }
    }
}

/// `f` on each RGBA pixel of the `(x, y, w, h)` rectangle, clipped to the frame.
fn fill(rgba: &mut [u8], (width, height): (usize, usize), (x, y, w, h): (usize, usize, usize, usize), f: impl Fn(&mut [u8])) {
    for yy in y..(y + h).min(height) {
        for xx in x..(x + w).min(width) {
            let p = (yy * width + xx) * 4;
            f(&mut rgba[p..p + 4]);
        }
    }
}

/// "60 FPS 100%": frames emulated per second and the share of the game's
/// own rate (`Game::fps`, 50 on PAL).
pub fn speed_line(fps: f64, target: f64) -> String {
    let pct = if target > 0.0 { fps / target * 100.0 } else { 0.0 };
    format!("{fps:.0} FPS {pct:.0}%")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_line_reports_share_of_target() {
        assert_eq!(speed_line(60.1, 60.1), "60 FPS 100%");
        assert_eq!(speed_line(30.0, 60.0), "30 FPS 50%");
        assert_eq!(speed_line(50.0, 0.0), "50 FPS 0%");
    }

    #[test]
    fn draws_inside_the_frame_and_clips_at_the_edge() {
        let (w, h) = (16, 10);
        let mut rgba = vec![0xFFu8; w * h * 4];
        draw(&mut rgba, w, h, &["SOUND OFF"], 1);
        // Box darkened the corner, text stayed light, nothing panicked past the edge.
        assert_eq!(rgba[(2 * w + 2) * 4], 0x3F);
        assert!(rgba.iter().step_by(4).any(|&r| r == 0xF8));
    }
}
