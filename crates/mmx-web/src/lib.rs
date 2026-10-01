//! mmx-web: the browser frontend. Rust side is a thin `Emu` over
//! [`mmx_core::Game`]; DOM, canvas, audio and keyboard live in `site/app.js`.
//! Modelled on z2rs `crates/z2-web`, shrunk hard. See `docs/web.md`.

use mmx_core::{
    Game, Pads, BTN_A, BTN_B, BTN_DOWN, BTN_L, BTN_LEFT, BTN_R, BTN_RIGHT, BTN_SELECT, BTN_START,
    BTN_UP, BTN_X, BTN_Y,
};
use wasm_bindgen::prelude::*;

#[cfg(feature = "netplay")]
mod net;

/// `KeyboardEvent.code` -> (player 1|2, pad bit). Player 1 on the arrows and
/// ZXCV, player 2 on WASD and the letters around G.
const KEYS: &[(&str, u8, u16)] = &[
    ("ArrowUp", 1, BTN_UP),
    ("ArrowDown", 1, BTN_DOWN),
    ("ArrowLeft", 1, BTN_LEFT),
    ("ArrowRight", 1, BTN_RIGHT),
    ("KeyZ", 1, BTN_B),
    ("KeyX", 1, BTN_Y),
    ("KeyC", 1, BTN_A),
    ("KeyV", 1, BTN_X),
    ("KeyQ", 1, BTN_L),
    ("KeyE", 1, BTN_R),
    ("Enter", 1, BTN_START),
    ("ShiftLeft", 1, BTN_SELECT),
    ("ShiftRight", 1, BTN_SELECT),
    ("KeyW", 2, BTN_UP),
    ("KeyS", 2, BTN_DOWN),
    ("KeyA", 2, BTN_LEFT),
    ("KeyD", 2, BTN_RIGHT),
    ("KeyG", 2, BTN_B),
    ("KeyF", 2, BTN_Y),
    ("KeyH", 2, BTN_A),
    ("KeyT", 2, BTN_X),
    ("KeyR", 2, BTN_L),
    ("KeyY", 2, BTN_R),
    ("KeyP", 2, BTN_START),
    ("KeyO", 2, BTN_SELECT),
];

/// Map a `KeyboardEvent.code` to `(player << 16) | pad_bit`, 0 if unbound.
/// With `coop` off the player-2 keys are ignored. The two tables are disjoint.
#[wasm_bindgen]
pub fn key_to_pad(code: &str, coop: bool) -> u32 {
    KEYS.iter()
        .find(|(c, p, _)| *c == code && (coop || *p == 1))
        .map_or(0, |&(_, p, bit)| (u32::from(p) << 16) | u32::from(bit))
}

/// 0x00RRGGBB -> RGBA8888 (opaque), resizing `dst`.
pub fn xrgb_to_rgba(src: &[u32], dst: &mut Vec<u8>) {
    dst.resize(src.len() * 4, 0);
    for (px, out) in src.iter().zip(dst.chunks_exact_mut(4)) {
        let [_, r, g, b] = px.to_be_bytes();
        out.copy_from_slice(&[r, g, b, 0xFF]);
    }
}

#[wasm_bindgen]
pub struct Emu {
    game: Game,
    rgba: Vec<u8>,
    width: usize,
    height: usize,
    audio_rate: Option<u32>,
    /// Audio already taken from the game by `net_tick_audio`, for `audio_take`.
    pcm: Vec<(f32, f32)>,
    /// ROM bytes: an online session rebuilds the game from power-on.
    #[cfg(feature = "netplay")]
    rom: Vec<u8>,
    #[cfg(feature = "netplay")]
    net: Option<mmx_core::netgame::NetPlay<mmx_net::MatchboxTransport>>,
    /// The last dropped session's log (`net_record`).
    #[cfg(feature = "netplay")]
    net_log: String,
}

#[wasm_bindgen]
impl Emu {
    #[wasm_bindgen(constructor)]
    pub fn new(rom: &[u8]) -> Result<Emu, JsValue> {
        let game = Game::from_rom(rom.to_vec()).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let mut emu = Emu {
            game,
            rgba: Vec::new(),
            width: 0,
            height: 0,
            audio_rate: None,
            pcm: Vec::new(),
            #[cfg(feature = "netplay")]
            rom: rom.to_vec(),
            #[cfg(feature = "netplay")]
            net: None,
            #[cfg(feature = "netplay")]
            net_log: String::new(),
        };
        emu.refresh_frame();
        Ok(emu)
    }

    /// Run one frame with these pads (see `mmx_core::Pads` for bit layout).
    pub fn step(&mut self, p1: u16, p2: u16) {
        self.game.step(Pads { p1, p2 });
        self.refresh_frame();
    }

    /// Zero-copy view: `new Uint8ClampedArray(wasm.memory.buffer, ptr, len)`.
    /// Re-read after every `step` (the buffer can move when the width changes).
    pub fn frame_ptr(&self) -> *const u8 {
        self.rgba.as_ptr()
    }
    pub fn frame_len(&self) -> usize {
        self.rgba.len()
    }
    pub fn width(&self) -> usize {
        self.width
    }
    pub fn height(&self) -> usize {
        self.height
    }
    /// Copying variant for automation scripts.
    pub fn frame_rgba(&self) -> Vec<u8> {
        self.rgba.clone()
    }

    /// Interleaved stereo f32 since the last call.
    pub fn audio_take(&mut self) -> Vec<f32> {
        let mut pcm = std::mem::take(&mut self.pcm);
        pcm.extend(self.game.take_audio());
        pcm.into_iter().flat_map(|(l, r)| [l, r]).collect()
    }
    pub fn set_audio_rate(&mut self, hz: u32) {
        self.game.set_audio_rate(hz);
        self.audio_rate = Some(hz);
        // audio may start (first key press) after net_connect: the session's
        // rebuilt game must still render at the device rate
        #[cfg(feature = "netplay")]
        if let Some(n) = self.net.as_mut() {
            n.audio_rate = Some(hz);
        }
    }
    /// Emulated frames per second (50.007 on a PAL ROM, 60.0988 NTSC).
    pub fn fps(&self) -> f64 {
        self.game.fps()
    }

    pub fn save_state(&self) -> Vec<u8> {
        self.game.save_state()
    }
    pub fn load_state(&mut self, s: &[u8]) -> Result<(), JsValue> {
        self.game.load_state(s).map_err(|e| JsValue::from_str(&e))?;
        self.refresh_frame();
        Ok(())
    }

    /// Mega Man X3 (USA) image: P2 is drawn as X3 Zero and X swings the
    /// saber (docs/zero.md "Playing as Zero"). Load it before a netplay
    /// connect: both peers need it (the handshake refuses a mismatch).
    pub fn set_x3_rom(&mut self, rom: &[u8]) -> Result<(), JsValue> {
        self.game.set_x3_rom(rom).map_err(|e| JsValue::from_str(&e))
    }

    /// Player 1 plays as Zero (needs [`Emu::set_x3_rom`] first). Before a
    /// netplay connect: both peers must agree.
    pub fn set_p1_zero(&mut self, on: bool) -> Result<(), JsValue> {
        self.game.set_p1_zero(on).map_err(|e| JsValue::from_str(&e))
    }

    /// Widescreen preset: `off`, `16:10`, `16:9`, `21:9` or `N` tiles per
    /// side (docs/widescreen.md). Display only, fine during netplay. The
    /// frame is wide from the next `step`. Returns false on a bad name.
    pub fn set_widescreen(&mut self, name: &str) -> bool {
        mmx_core::Widescreen::parse(name).map(|w| self.game.set_widescreen(w)).is_some()
    }

    /// Wide gameplay (docs/widescreen.md "Wide gameplay"): enemies spawn at
    /// the wide edges. Changes the game: set it before a netplay connect,
    /// both peers must agree (the handshake refuses a mismatch). Turned on
    /// mid-stage, the margins' columns are spawned at once.
    pub fn set_wide_gameplay(&mut self, on: bool) {
        self.game.set_wide_gameplay(on);
        if on {
            mmx_core::wide_gameplay::sweep(&mut self.game);
        }
    }

    /// CPU overclock `off`, `1.5` or `2` (docs/widescreen.md "Slowdown and
    /// overclock"): removes the game's slowdown. Changes the game like wide
    /// gameplay (set before a netplay connect; both peers must agree).
    /// Returns false on a bad name.
    pub fn set_overclock(&mut self, name: &str) -> bool {
        name.parse().map(|o| self.game.set_overclock(o)).is_ok()
    }

    /// Co-op on/off. Throws (and stays off) when this ROM lacks a hook
    /// (see [`Emu::hook_report`]).
    pub fn set_coop(&mut self, on: bool) -> Result<(), JsValue> {
        if on && !self.game.traps.is_trapped(self.game.hook(mmx_core::coop::PLAYER_OAM)) {
            mmx_core::coop::register_coop_traps(&mut self.game).map_err(|e| JsValue::from_str(&e))?;
        }
        self.game.coop.enabled = on;
        Ok(())
    }

    /// Where this ROM keeps the co-op hooks, one per line: empty when all
    /// sit where the PAL dump has them (docs/rom-variants.md).
    pub fn hook_report(&self) -> String {
        if self.game.hooks().all_as_pal() { String::new() } else { self.game.hook_report().join("\n") }
    }
    pub fn frame_count(&self) -> u64 {
        self.game.frame_count()
    }
    /// Copy of WRAM ($7E:0000-$7F:FFFF) for automation.
    pub fn wram(&self) -> Vec<u8> {
        self.game.wram().to_vec()
    }

    pub fn rom_info_json(&self) -> String {
        let i = self.game.rom_info();
        let sha: String = i.sha256.iter().map(|b| format!("{b:02x}")).collect();
        format!(
            r#"{{"title":"{}","version":{},"sha256":"{sha}","crc32":"{:08x}","had_copier_header":{}}}"#,
            json_escape(&i.title),
            i.version,
            i.crc32,
            i.had_copier_header
        )
    }
}

impl Emu {
    fn refresh_frame(&mut self) {
        let (px, w, h) = self.game.frame();
        xrgb_to_rgba(px, &mut self.rgba);
        self.width = w;
        self.height = h;
    }
}

// ponytail: header titles are 21 ASCII bytes; no serde for one string.
fn json_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '"' | '\\' => format!("\\{c}"),
            c if c.is_control() || !c.is_ascii() => format!("\\u{:04x}", u32::from(c) & 0xFFFF),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solo_keys_are_player_one() {
        assert_eq!(key_to_pad("ArrowLeft", false), 0x1_0000 | u32::from(BTN_LEFT));
        assert_eq!(key_to_pad("KeyZ", false), 0x1_0000 | u32::from(BTN_B));
        assert_eq!(key_to_pad("KeyQ", false), 0x1_0000 | u32::from(BTN_L));
        assert_eq!(key_to_pad("KeyE", false), 0x1_0000 | u32::from(BTN_R));
        assert_eq!(key_to_pad("KeyA", false), 0);
        assert_eq!(key_to_pad("ShiftRight", false), 0x1_0000 | u32::from(BTN_SELECT));
        assert_eq!(key_to_pad("KeyW", false), 0, "P2 keys ignored solo");
        assert_eq!(key_to_pad("F5", false), 0);
    }

    #[test]
    fn coop_gives_wasd_to_player_two() {
        assert_eq!(key_to_pad("KeyA", true), 0x2_0000 | u32::from(BTN_LEFT));
        assert_eq!(key_to_pad("KeyS", true), 0x2_0000 | u32::from(BTN_DOWN));
        assert_eq!(key_to_pad("KeyP", true), 0x2_0000 | u32::from(BTN_START));
        assert_eq!(key_to_pad("KeyY", true), 0x2_0000 | u32::from(BTN_R));
        assert_eq!(key_to_pad("KeyQ", true), 0x1_0000 | u32::from(BTN_L));
        // the two tables never share a key
        for &(c, p, _) in KEYS {
            assert!(!KEYS.iter().any(|k| k.0 == c && k.1 != p), "{c}");
        }
        assert_eq!(key_to_pad("Enter", true), 0x1_0000 | u32::from(BTN_START));
    }

    #[test]
    fn no_key_binds_two_buttons_for_one_player() {
        for &(c, p, _) in KEYS {
            assert_eq!(KEYS.iter().filter(|k| k.0 == c && k.1 == p).count(), 1, "{c}");
        }
    }

    #[test]
    fn rgba_conversion() {
        let mut out = vec![9; 3];
        xrgb_to_rgba(&[0x00FF_8001, 0xAB12_3456], &mut out);
        assert_eq!(out, [0xFF, 0x80, 0x01, 0xFF, 0x12, 0x34, 0x56, 0xFF]);
    }

    #[test]
    fn escapes_json() {
        assert_eq!(json_escape(r#"MEGA"MAN\X"#), r#"MEGA\"MAN\\X"#);
    }
}
