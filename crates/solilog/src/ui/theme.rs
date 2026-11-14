//! Colors, shared by every view.

pub const BACKGROUND: u32 = 0x1c1c1f;
pub const SIDEBAR: u32 = 0x19191c;
pub const SURFACE: u32 = 0x26262b;
pub const BORDER: u32 = 0x3a3a42;
pub const DIVIDER: u32 = 0x2c2c31;
pub const HOVER: u32 = 0x2c2c31;
pub const HIGHLIGHT: u32 = 0x34343b;
/// Issues selected for a command, and the highlighted one among them.
pub const SELECTION: u32 = 0x272b40;
pub const SELECTION_HIGHLIGHT: u32 = 0x32385a;
pub const TEXT: u32 = 0xe6e6e6;
pub const TEXT_MUTED: u32 = 0x7a7a85;
pub const ACCENT: u32 = 0x7c8cff;
pub const ERROR: u32 = 0xeb5757;

/// Chip color of a label: its own, or muted when it has none.
pub fn label_color(color: Option<solilog_core::LabelColor>) -> u32 {
    use solilog_core::LabelColor::*;
    match color {
        None => TEXT_MUTED,
        Some(Red) => 0xeb6f6f,
        Some(Orange) => 0xeb9a5c,
        Some(Yellow) => 0xe5c453,
        Some(Green) => 0x62c47a,
        Some(Teal) => 0x4cc0b6,
        Some(Blue) => 0x5aa0ee,
        Some(Purple) => 0xa27cf0,
        Some(Pink) => 0xe377b6,
        Some(Gray) => 0xa0a0aa,
    }
}
