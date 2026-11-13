//! grok-pi's welcome brand, shared by the standalone `grok-pi` and grok-fork launching `grok-pi-acp`.

/// Matches Pi's static logo (`print_static_logo`): two-space indent + block art.
/// Kept as plain full-block art so it remains legible on terminals that cannot
/// render Grok's default braille logo. The welcome logo renderer pads rows to a
/// common visual width so per-line centering does not drift the glyph.
pub const PI_LOGO: &str = "\
  ██████\n\
  ██  ██\n\
  ████  ██\n\
  ██    ██\n\
";

pub const PI_WELCOME_SUBTITLE: &str = "Pi agent core in Grok Build's native terminal UI";

pub const PI_CHANGELOG_URL: &str = "https://github.com/Dwsy/grok-pi/blob/main/CHANGELOG.MD";
