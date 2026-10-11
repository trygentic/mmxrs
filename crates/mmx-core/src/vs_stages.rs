//! ROM-backed locations offered by VS mode.
//!
//! Each entry uses a real X1 level ID.  The menu names describe the opening
//! screen of a level, which can be loaded by the cartridge's normal stage
//! loader; no tile maps or collision data are fabricated here.

use crate::{ram, Game, Pads, BTN_START};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Arena {
    /// Stable identifier for configuration and the native stage picker.
    pub id: &'static str,
    pub name: &'static str,
    /// `$7E1F7A`, Mega Man X (USA) level ID.
    pub stage: u8,
    /// Initial horizontal offsets from the cartridge's safe P1 beam-in point.
    pub spawn_offsets: [i16; 4],
}

const OFFSETS: [i16; 4] = [0, 24, -24, 48];

/// Locations with distinct real backgrounds, terrain and music.
pub const ARENAS: &[Arena] = &[
    Arena { id: "highway", name: "Abandoned Highway", stage: 0x00, spawn_offsets: OFFSETS },
    Arena { id: "octopus", name: "Ocean Base", stage: 0x01, spawn_offsets: OFFSETS },
    Arena { id: "chameleon", name: "Forest Approach", stage: 0x02, spawn_offsets: OFFSETS },
    Arena { id: "armadillo", name: "Mine Entrance", stage: 0x03, spawn_offsets: OFFSETS },
    Arena { id: "mammoth", name: "Factory Floor", stage: 0x04, spawn_offsets: OFFSETS },
    Arena { id: "eagle", name: "Airport Runway", stage: 0x05, spawn_offsets: OFFSETS },
    Arena { id: "mandrill", name: "Power Plant", stage: 0x06, spawn_offsets: OFFSETS },
    Arena { id: "kuwanger", name: "Tower Entrance", stage: 0x07, spawn_offsets: OFFSETS },
    Arena { id: "penguin", name: "Snow Base", stage: 0x08, spawn_offsets: OFFSETS },
];

pub fn arena(id: &str) -> Option<&'static Arena> {
    ARENAS.iter().find(|arena| arena.id.eq_ignore_ascii_case(id))
}

/// Boot a fresh cartridge into the selected ROM stage.  The cartridge's own
/// loader must reach gameplay before this returns, so stage art and collision
/// always come from the selected ROM rather than a synthetic arena.
pub fn start_stage(game: &mut Game, arena: &Arena) -> Result<(), String> {
    let ix = ram::wram_index;
    let mut loading_selected_stage = false;
    // A cold X1 cartridge plays the CAPCOM logo and title before exposing the
    // short `D2=2, D3=$16` new-game transition. Repeated Start taps let the
    // ROM finish those screens at its own pace.
    for frame in 0..3600 {
        if loading_selected_stage {
            // The new-game transition normally writes level 0 (Highway).
            // Replace that byte while the ROM's loader reads it, then let the
            // loader build its own PPU, collision map and object tables.
            game.wram_mut()[ix(ram::LEVEL_ID)] = arena.stage;
        }
        let start = !loading_selected_stage && frame >= 720 && frame % 100 < 4;
        game.step(Pads { p1: if start { BTN_START } else { 0 }, p2: 0 });
        let w = game.wram();
        let mode = w[ix(ram::MODE_D2)];
        let phase = w[ix(ram::STAGE_PHASE)];
        if w[ix(ram::MODE_D1)] == 2 && mode == 2 && phase == 0x16 {
            loading_selected_stage = true;
        }
        if loading_selected_stage && mode == ram::D2_IN_STAGE && phase == ram::PHASE_GAMEPLAY {
            let loaded = w[ix(ram::LEVEL_ID)];
            if loaded != arena.stage {
                return Err(format!("VS stage loader chose level {loaded:02X} instead of {:02X}", arena.stage));
            }
            return Ok(());
        }
    }
    Err(format!("VS stage {} did not reach gameplay within 3600 ROM frames", arena.id))
}
