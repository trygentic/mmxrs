//! macOS GameController-framework gamepads (ported from z2rs's `gc_pad.rs`).
//!
//! gilrs's macOS backend reads IOKit HID. Some pads never show up there (a
//! Switch Pro Controller over Bluetooth) or enumerate but deliver no input
//! (Apple-driven Xbox pads); all of them, plus DualShock 4 / DualSense, reach
//! `GCController`. This snapshots every connected extended gamepad once per
//! frame into SNES pad bits with the same positional layout as the gilrs path
//! (`keys::GC_BUTTONS`, `keys::stick_bits`).
//!
//! Discovery notifications arrive on the main run loop, which winit pumps, so
//! plain polling of `GCController::controllers()` stays current.

use crate::keys;
use objc2_game_controller::{GCController, GCControllerButtonInput, GCControllerDirectionPad, GCDevice};

/// One connected controller this frame.
#[derive(Debug, Clone)]
pub struct GcPad {
    /// The controller object's address: stable while it stays connected.
    pub key: usize,
    pub name: String,
    pub bits: u16,
}

fn pressed(b: &GCControllerButtonInput) -> bool {
    // SAFETY: plain property read on a live framework object.
    unsafe { b.isPressed() }
}

fn dpad_bits(d: &GCControllerDirectionPad) -> u16 {
    // SAFETY: plain property reads on a live framework object.
    let (up, down, left, right) = unsafe { (d.up(), d.down(), d.left(), d.right()) };
    [(up, mmx_core::BTN_UP), (down, mmx_core::BTN_DOWN), (left, mmx_core::BTN_LEFT), (right, mmx_core::BTN_RIGHT)]
        .iter()
        .filter(|(b, _)| pressed(b))
        .fold(0, |a, (_, bit)| a | bit)
}

/// `"Pro Controller (Switch Pro Controller)"`: vendor name plus category.
fn name(c: &GCController) -> String {
    // SAFETY: property reads on a live framework object.
    let (vendor, cat) = unsafe { (c.vendorName(), c.productCategory()) };
    let vendor = vendor.map_or_else(|| "unnamed controller".into(), |s| s.to_string());
    format!("{vendor} ({cat})")
}

/// Let pads reach us while another app is frontmost (the `--pads` diagnostic
/// runs from a terminal that keeps focus).
pub fn monitor_background(on: bool) {
    // SAFETY: class property setter, main thread.
    unsafe { GCController::setShouldMonitorBackgroundEvents(on) }
}

/// Every connected controller with an extended gamepad profile.
pub fn snapshot() -> Vec<GcPad> {
    let mut out = Vec::new();
    // SAFETY: class/property reads on the main thread (the winit loop).
    unsafe {
        for c in GCController::controllers().iter() {
            let Some(gp) = c.extendedGamepad() else {
                continue;
            };
            let options = gp.buttonOptions();
            let buttons = [
                Some(gp.buttonA()),
                Some(gp.buttonB()),
                Some(gp.buttonX()),
                Some(gp.buttonY()),
                Some(gp.leftShoulder()),
                Some(gp.rightShoulder()),
                Some(gp.leftTrigger()),
                Some(gp.rightTrigger()),
                Some(gp.buttonMenu()),
                options,
            ];
            let mut bits = buttons
                .iter()
                .zip(keys::GC_BUTTONS)
                .filter(|(b, _)| b.as_deref().is_some_and(pressed))
                .fold(0, |a, (_, (_, bit))| a | bit);
            bits |= dpad_bits(&gp.dpad());
            let stick = gp.leftThumbstick();
            // GC's Y axis is up-positive, like gilrs.
            bits |= keys::stick_bits(stick.xAxis().value(), stick.yAxis().value());
            out.push(GcPad {
                key: &*c as *const GCController as usize,
                name: name(&c),
                bits,
            });
        }
    }
    out
}
