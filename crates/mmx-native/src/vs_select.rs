//! Local VS character and arena selection. Drawn into the same low-resolution
//! pixel surface as the game so controller selection works before ROM startup.

use mmx_core::{bone_animation::BoneFrame, Character, BTN_A, BTN_B, BTN_DOWN, BTN_LEFT, BTN_RIGHT, BTN_SELECT, BTN_START, BTN_UP, BTN_X, BTN_Y};

const COLORS: [u32; 4] = [0x52B8FF, 0xFF6471, 0x61DDA0, 0xC495FF];
const COLS: usize = 5;
const GRID_X: i32 = 15;
const GRID_Y: i32 = 47;
const CELL_W: i32 = 46;
const CELL_H: i32 = 28;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Characters, Arena, Done }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VsChoice {
    pub characters: [Character; 4],
    pub stage_index: usize,
}

pub struct VsSelect {
    players: usize,
    cursors: [usize; 4],
    locked: [bool; 4],
    previous: [u16; 4],
    held_direction: [u16; 4],
    held_frames: [u8; 4],
    stage_index: usize,
    stage_names: Vec<String>,
    portraits: Vec<Option<BoneFrame>>,
    error: Option<String>,
    phase: Phase,
    ticks: u64,
}

impl VsSelect {
    pub fn new(players: usize, initial: [Character; 4], stage_names: Vec<String>, initial_stage: usize) -> Self {
        let mut cursors = [0; 4];
        for (index, character) in initial.into_iter().enumerate() {
            cursors[index] = Character::ALL.iter().position(|&c| c == character).unwrap_or(0);
        }
        Self {
            players: players.clamp(2, 4), cursors, locked: [false; 4], previous: [0; 4],
            held_direction: [0; 4], held_frames: [0; 4],
            stage_index: initial_stage.min(stage_names.len().saturating_sub(1)),
            stage_names, portraits: Vec::new(), error: None, phase: Phase::Characters, ticks: 0,
        }
    }

    pub fn set_portraits(&mut self, portraits: Vec<Option<BoneFrame>>) {
        self.portraits = portraits;
    }

    pub fn choice(&self) -> Option<VsChoice> {
        (self.phase == Phase::Done).then(|| VsChoice {
            characters: self.cursors.map(|i| Character::ALL[i]),
            stage_index: self.stage_index,
        })
    }

    pub fn retry_arena(&mut self, error: String) {
        self.phase = Phase::Arena;
        self.error = Some(error);
    }

    pub fn step(&mut self, pads: [u16; 4]) -> Option<VsChoice> {
        self.ticks += 1;
        let mut pressed = std::array::from_fn::<_, 4, _>(|i| pads[i] & !self.previous[i]);
        for i in 0..self.players {
            let direction = pads[i] & (BTN_UP | BTN_DOWN | BTN_LEFT | BTN_RIGHT);
            if direction == 0 || direction != self.held_direction[i] {
                self.held_frames[i] = 0;
            } else {
                self.held_frames[i] = self.held_frames[i].saturating_add(1);
                if self.held_frames[i] >= 12 && (self.held_frames[i] - 12) % 5 == 0 {
                    pressed[i] |= direction;
                }
            }
            self.held_direction[i] = direction;
        }
        self.previous = pads;
        match self.phase {
            Phase::Characters => {
                for (i, &bits) in pressed.iter().enumerate().take(self.players) {
                    if self.locked[i] {
                        if bits & BTN_SELECT != 0 { self.locked[i] = false; }
                        continue;
                    }
                    let mut row = self.cursors[i] / COLS;
                    let mut col = self.cursors[i] % COLS;
                    if bits & BTN_UP != 0 { row = (row + 3) % 4; }
                    else if bits & BTN_DOWN != 0 { row = (row + 1) % 4; }
                    if bits & BTN_LEFT != 0 { col = (col + COLS - 1) % COLS; }
                    else if bits & BTN_RIGHT != 0 { col = (col + 1) % COLS; }
                    let next = row * COLS + col;
                    if next < Character::ALL.len() { self.cursors[i] = next; }
                    if bits & (BTN_A | BTN_B | BTN_X | BTN_Y | BTN_START) != 0 {
                        self.locked[i] = true;
                    }
                }
                if self.locked[..self.players].iter().all(|&yes| yes) {
                    self.phase = Phase::Arena;
                    self.previous = pads;
                }
            }
            Phase::Arena => {
                // Player one picks the arena once everyone has locked a fighter.
                let bits = pressed[0];
                if bits & (BTN_LEFT | BTN_UP) != 0 && !self.stage_names.is_empty() {
                    self.stage_index = (self.stage_index + self.stage_names.len() - 1) % self.stage_names.len();
                    self.error = None;
                } else if bits & (BTN_RIGHT | BTN_DOWN) != 0 && !self.stage_names.is_empty() {
                    self.stage_index = (self.stage_index + 1) % self.stage_names.len();
                    self.error = None;
                }
                if bits & BTN_SELECT != 0 {
                    self.phase = Phase::Characters;
                    self.locked[0] = false;
                } else if bits & (BTN_A | BTN_B | BTN_X | BTN_Y | BTN_START) != 0 {
                    self.phase = Phase::Done;
                }
            }
            Phase::Done => {}
        }
        self.choice()
    }

    pub fn draw(&self, rgba: &mut [u8], width: usize, height: usize) {
        if rgba.len() < width * height * 4 { return; }
        let mut canvas = Canvas { rgba, width: width as i32, height: height as i32 };
        let ox = (width as i32 - 256) / 2;
        canvas.rect(0, 0, width as i32, height as i32, 0x090E25);
        for y in (0..height as i32).step_by(8) {
            canvas.rect(0, y, width as i32, 1, 0x142348);
        }
        canvas.rect(ox + 2, 2, 252, 220, 0x233A70);
        canvas.rect(ox + 4, 4, 248, 216, 0x10172F);
        canvas.rect(ox + 4, 4, 248, 31, 0x203C76);
        canvas.rect(ox + 4, 33, 248, 2, 0xF2C765);
        canvas.text(ox + 17, 9, "MEGA MAN X", 2, 0xFFFFFF);
        canvas.text(ox + 179, 8, "VS", 4, 0xFFD772);
        canvas.text(ox + 17, 27, "CHOOSE YOUR FIGHTER", 1, 0x8DE9FF);

        match self.phase {
            Phase::Characters => self.draw_characters(&mut canvas, ox),
            Phase::Arena | Phase::Done => self.draw_arena(&mut canvas, ox),
        }
    }

    fn draw_characters(&self, c: &mut Canvas<'_>, ox: i32) {
        for (index, &character) in Character::ALL.iter().enumerate() {
            let x = ox + GRID_X + (index % COLS) as i32 * CELL_W;
            let y = GRID_Y + (index / COLS) as i32 * CELL_H;
            let tint = fighter_color(index);
            c.rect(x, y, CELL_W - 2, CELL_H - 2, 0x5686B8);
            c.rect(x + 2, y + 2, CELL_W - 6, CELL_H - 6, tint);
            c.rect(x + 4, y + 3, 16, 20, 0x10172F);
            if let Some(Some(portrait)) = self.portraits.get(index) {
                c.portrait(x + 4, y + 3, 16, 20, portrait);
            } else {
                c.text(x + 6, y + 7, &character.label().chars().next().unwrap_or('X').to_string(), 2, 0xFFF1C8);
            }
            let abbr: String = character.label().chars().filter(|ch| ch.is_ascii_alphabetic()).take(5).collect();
            c.text(x + 21, y + 9, &abbr, 1, 0xFFFFFF);
            for player in 0..self.players {
                if self.cursors[player] == index {
                    let border = COLORS[player];
                    let offset = player as i32;
                    c.frame(x - 1 - offset, y - 1 - offset, CELL_W + offset * 2, CELL_H + offset * 2, border);
                    c.text(x + 2 + player as i32 * 10, y - 5, &(player + 1).to_string(), 1, border);
                }
            }
        }
        for player in 0..self.players {
            let x = ox + 12 + (player % 2) as i32 * 121;
            let y = 165 + (player / 2) as i32 * 25;
            let color = COLORS[player];
            c.rect(x, y, 111, 22, color);
            c.rect(x + 2, y + 2, 107, 18, 0x142242);
            c.text(x + 5, y + 4, &format!("P{}", player + 1), 2, color);
            c.text(x + 30, y + 4, Character::ALL[self.cursors[player]].label(), 1, 0xFFFFFF);
            c.text(x + 30, y + 12, if self.locked[player] { "READY" } else { "SELECT" }, 1,
                if self.locked[player] { 0x7BF0AA } else { 0xFFCF69 });
        }
        c.text(ox + 18, 215, "MOVE  D-PAD    LOCK  ATTACK    BACK  SELECT", 1, 0x9AB9DE);
    }

    fn draw_arena(&self, c: &mut Canvas<'_>, ox: i32) {
        c.text(ox + 53, 44, "CHOOSE THE ARENA", 2, 0xFFD772);
        for player in 0..self.players {
            let x = ox + 14 + (player % 2) as i32 * 122;
            let y = 75 + (player / 2) as i32 * 47;
            c.rect(x, y, 110, 40, COLORS[player]);
            c.rect(x + 2, y + 2, 106, 36, 0x152749);
            c.text(x + 5, y + 7, &format!("P{}", player + 1), 2, COLORS[player]);
            c.text(x + 29, y + 10, Character::ALL[self.cursors[player]].label(), 1, 0xFFFFFF);
        }
        c.rect(ox + 13, 173, 230, 28, 0xC8964B);
        c.rect(ox + 15, 175, 226, 24, 0x192A50);
        let name = self.stage_names.get(self.stage_index).map_or("ARENA", String::as_str);
        c.text(ox + 22, 181, "<", 2, 0xFFD772);
        c.text(ox + 229, 181, ">", 2, 0xFFD772);
        let text_width = (name.len() as i32 * 4 * 2).min(194);
        c.text(ox + 128 - text_width / 2, 181, name, 2, 0xFFFFFF);
        if let Some(error) = &self.error {
            c.text(ox + 10, 211, &error.chars().take(58).collect::<String>(), 1, 0xFF7777);
        } else {
            c.text(ox + 20, 211, "P1  CHANGE  D-PAD    START  ATTACK", 1, 0x9AB9DE);
        }
    }
}

fn fighter_color(index: usize) -> u32 {
    const PALETTE: [u32; 8] = [0x1B579D, 0x963B38, 0x266E76, 0x794A8C, 0x92613C, 0x3C647F, 0x7F4258, 0x416C4E];
    PALETTE[index % PALETTE.len()]
}

pub fn draw_result(rgba: &mut [u8], width: usize, height: usize, status: mmx_core::vs::VsStatus) {
    if rgba.len() < width * height * 4 { return; }
    let mut c = Canvas { rgba, width: width as i32, height: height as i32 };
    let ox = (width as i32 - 256) / 2;
    c.rect(ox + 28, 72, 200, 82, 0xE6BD65);
    c.rect(ox + 31, 75, 194, 76, 0x17294E);
    let title = status.winner.map_or("DRAW".to_owned(), |i| format!("PLAYER {} WINS", i + 1));
    let center = ox + 128 - (title.len() as i32 * 8 / 2);
    c.text(center, 83, &title, 2, 0xFFD772);
    c.text(ox + 76, 106, &format!("ROUND {}", status.round), 2, 0xFFFFFF);
    for i in 0..status.players.min(4) {
        let x = ox + 51 + (i % 2) as i32 * 107;
        let y = 125 + (i / 2) as i32 * 10;
        c.text(x, y, &format!("P{}  {}", i + 1, status.scores[i]), 1, COLORS[i]);
    }
    c.text(ox + 74, 145, "START  NEXT ROUND", 1, 0x9DE5FF);
}

/// Result UI keeps its original game-pixel size above high-resolution art.
pub fn draw_result_scaled(rgba: &mut [u8], width: usize, height: usize,
                          status: mmx_core::vs::VsStatus, scale: u8) {
    let scale = usize::from(scale.max(1));
    if scale == 1 { return draw_result(rgba, width, height, status); }
    if rgba.len() < width * height * 4 { return; }
    let (w, h) = (width / scale, height / scale);
    let mut overlay = vec![0; w * h * 4];
    draw_result(&mut overlay, w, h, status);
    for (i, pixel) in overlay.chunks_exact(4).enumerate() {
        if pixel[3] == 0 { continue; }
        let (x, y) = (i % w, i / w);
        for yy in y * scale..(y + 1) * scale {
            for xx in x * scale..(x + 1) * scale {
                rgba[(yy * width + xx) * 4..][..4].copy_from_slice(pixel);
            }
        }
    }
}

struct Canvas<'a> { rgba: &'a mut [u8], width: i32, height: i32 }

impl Canvas<'_> {
    fn portrait(&mut self, x: i32, y: i32, width: i32, height: i32, frame: &BoneFrame) {
        let sw = i32::from(frame.width);
        let sh = i32::from(frame.height);
        if sw == 0 || sh == 0 { return; }
        let scale = (width as f32 / sw as f32).min(height as f32 / sh as f32);
        let dw = (sw as f32 * scale).round().max(1.0) as i32;
        let dh = (sh as f32 * scale).round().max(1.0) as i32;
        let left = x + (width - dw) / 2;
        let top = y + (height - dh) / 2;
        for py in 0..dh {
            for px in 0..dw {
                let sx = px * sw / dw;
                let sy = py * sh / dh;
                let src = ((sy * sw + sx) * 4) as usize;
                if frame.rgba[src + 3] < 128 { continue; }
                let color = (u32::from(frame.rgba[src]) << 16)
                    | (u32::from(frame.rgba[src + 1]) << 8)
                    | u32::from(frame.rgba[src + 2]);
                self.rect(left + px, top + py, 1, 1, color);
            }
        }
    }

    fn rect(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        let x0 = x.max(0); let y0 = y.max(0);
        let x1 = (x + w).min(self.width); let y1 = (y + h).min(self.height);
        for yy in y0..y1 {
            for xx in x0..x1 {
                let p = ((yy * self.width + xx) * 4) as usize;
                self.rgba[p] = (color >> 16) as u8;
                self.rgba[p + 1] = (color >> 8) as u8;
                self.rgba[p + 2] = color as u8;
                self.rgba[p + 3] = 255;
            }
        }
    }

    fn frame(&mut self, x: i32, y: i32, w: i32, h: i32, color: u32) {
        self.rect(x, y, w, 2, color);
        self.rect(x, y + h - 2, w, 2, color);
        self.rect(x, y, 2, h, color);
        self.rect(x + w - 2, y, 2, h, color);
    }

    fn text(&mut self, x: i32, y: i32, s: &str, scale: i32, color: u32) {
        for (i, ch) in s.chars().enumerate() {
            let glyph = glyph(ch.to_ascii_uppercase());
            for (dy, row) in glyph.into_iter().enumerate() {
                for dx in 0..3 {
                    if row & (4 >> dx) != 0 {
                        self.rect(x + i as i32 * 4 * scale + dx * scale, y + dy as i32 * scale, scale, scale, color);
                    }
                }
            }
        }
    }
}

fn glyph(c: char) -> [u8; 5] {
    match c {
        '0' => [7,5,5,5,7], '1' => [2,6,2,2,7], '2' => [7,1,7,4,7],
        '3' => [7,1,7,1,7], '4' => [5,5,7,1,1], '5' => [7,4,7,1,7],
        '6' => [7,4,7,5,7], '7' => [7,1,1,1,1], '8' => [7,5,7,5,7],
        '9' => [7,5,7,1,7], 'A' => [2,5,7,5,5], 'B' => [6,5,6,5,6],
        'C' => [7,4,4,4,7], 'D' => [6,5,5,5,6], 'E' => [7,4,7,4,7],
        'F' => [7,4,7,4,4], 'G' => [7,4,5,5,7], 'H' => [5,5,7,5,5],
        'I' => [7,2,2,2,7], 'J' => [1,1,1,5,7], 'K' => [5,5,6,5,5],
        'L' => [4,4,4,4,7], 'M' => [5,7,7,5,5], 'N' => [5,7,7,7,5],
        'O' => [7,5,5,5,7], 'P' => [7,5,7,4,4], 'Q' => [7,5,5,7,1],
        'R' => [6,5,6,5,5], 'S' => [7,4,7,1,7], 'T' => [7,2,2,2,2],
        'U' => [5,5,5,5,7], 'V' => [5,5,5,5,2], 'W' => [5,5,7,7,5],
        'X' => [5,5,2,5,5], 'Y' => [5,5,2,2,2], 'Z' => [7,1,2,4,7],
        '<' => [1,2,4,2,1], '>' => [4,2,1,2,4], '-' => [0,0,7,0,0],
        _ => [0; 5],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_players_can_lock_any_character_then_choose_stage() {
        let mut select = VsSelect::new(4, [Character::X; 4], vec!["HIGHWAY".into(), "EAGLE".into()], 0);
        for i in 0..4 {
            let mut pads = [0; 4];
            pads[i] = BTN_B;
            assert!(select.step(pads).is_none());
            select.step([0; 4]);
        }
        assert!(select.step([BTN_RIGHT, 0, 0, 0]).is_none());
        select.step([0; 4]);
        let choice = select.step([BTN_START, 0, 0, 0]).unwrap();
        assert_eq!(choice.stage_index, 1);
        assert_eq!(choice.characters[..4], [Character::X; 4]);
    }

    #[test]
    fn movement_and_cancel_are_independent_per_player() {
        let mut select = VsSelect::new(2, [Character::X, Character::Zero, Character::X, Character::X], vec![], 0);
        select.step([BTN_RIGHT, BTN_DOWN, 0, 0]);
        assert_eq!(select.cursors[0], 1);
        assert_eq!(select.cursors[1], 6);
        select.step([0; 4]);
        select.step([BTN_B, BTN_B, 0, 0]);
        assert_eq!(select.phase, Phase::Arena);
        select.step([0; 4]);
        select.step([BTN_SELECT, 0, 0, 0]);
        assert_eq!(select.phase, Phase::Characters);
        assert!(!select.locked[0]);
    }

    #[test]
    fn held_direction_repeats_after_a_short_delay() {
        let mut select = VsSelect::new(2, [Character::X; 4], vec![], 0);
        select.step([BTN_RIGHT, 0, 0, 0]);
        for _ in 0..11 { select.step([BTN_RIGHT, 0, 0, 0]); }
        assert_eq!(select.cursors[0], 1);
        select.step([BTN_RIGHT, 0, 0, 0]);
        assert_eq!(select.cursors[0], 2);
    }

    #[test]
    fn high_resolution_result_overlay_keeps_game_size() {
        let status = mmx_core::vs::VsStatus { winner: Some(1), round_over: true,
            scores: [0, 2, 1, 0], round: 3, players: 4 };
        let (mut native, mut hd) = (vec![0; 256 * 224 * 4], vec![0; 1024 * 896 * 4]);
        draw_result(&mut native, 256, 224, status);
        draw_result_scaled(&mut hd, 1024, 896, status, 4);
        for y in 0..896 { for x in 0..1024 {
            assert_eq!(&hd[(y * 1024 + x) * 4..][..4], &native[(y / 4 * 256 + x / 4) * 4..][..4]);
        } }
    }
}
