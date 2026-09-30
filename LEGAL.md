# Legal posture

- mmxrs is licensed GPL-3.0-only (see LICENSE) because it forks the jgenesis
  emulator core (GPL-3.0) into `vendor/jgenesis`.
- No Capcom-owned bytes are in this repository: no ROM, no graphics, no music,
  no text, no test fixtures derived from them. Bring your own Mega Man X (USA)
  cartridge dump and point `MMX_ROM` at it. Tools refuse to copy it into the tree.
- `vendor/matchbox_socket` is MIT OR Apache-2.0. `crates/mmx-net` and
  `tools/mmx-signal` are copied from z2rs (MIT) and relicensed here under GPL-3.0
  as permitted.
- snesrecomp and MegaManXSNESRecomp are PolyForm Noncommercial; nothing from
  them is copied. The MMX1-Disassembly repo has no licence; we use it only to
  read addresses, never to copy text.
