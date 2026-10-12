//! Vile's on-foot PNG is private art and not in a release: a player who picks
//! Vile without it must still get a game (ROM poses), not exit code 2.
//! Skipped unless `MMX_ROM` is set.

use std::process::Command;

#[test]
fn vile_runs_on_rom_poses_without_the_png() {
    let Some(rom) = std::env::var_os("MMX_ROM") else {
        return eprintln!("MMX_ROM not set; skipping Vile launch test");
    };
    let dir = std::env::temp_dir().join(format!("mmx-vile-no-png-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let missing = dir.join("no-such-vile.png");
    let out = Command::new(env!("CARGO_BIN_EXE_mmxrs"))
        .current_dir(&dir)
        .arg("--rom")
        .arg(&rom)
        .args(["--headless", "--frames", "120", "--p1-character", "vile", "--vile-sprite"])
        .arg(&missing)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(out.status.success(), "exit {:?}\n{stderr}", out.status.code());
    assert!(stderr.contains("using his ROM poses"), "{stderr}");
}
