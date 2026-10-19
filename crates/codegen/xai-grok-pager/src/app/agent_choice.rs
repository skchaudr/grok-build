//! Named agent commands (`--agent-choice NAME=CMD`, or `[[agent_choice]]` in config.toml) and
//! the new-session machine picker.
//!
//! One pager can offer several machines. The first choice is the default. With two or more,
//! creating a session asks which name to use before `session/new` is sent.
//!
//! The list comes from one of two places, never merged: every `--agent-choice` flag on the
//! command line, or, when there are none, the `[[agent_choice]]` tables of the effective
//! config (`~/.grok/config.toml` and the managed layers above it). Config tables keep their file
//! order, so the first table is the default, exactly like the first flag.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

/// One machine the pager can start a session on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentChoice {
    pub name: String,
    pub cmd: String,
}

/// Why the picker was opened, so confirm can resume that new-session path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MachinePickIntent {
    NewSession,
    DashboardNewAgent,
    DashboardPrompt { text: String, attach: bool },
}

/// Result of asking the picker whether a new session may proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewSessionPick {
    /// No named choices. The existing single `--agent-cmd` path is unchanged.
    Skip,
    /// Show the picker. The session is not created yet.
    Ask,
    /// Use this machine for the session about to be created.
    Use(AgentChoice),
}

/// Cursor over the named machines. Default selection is the first choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MachinePicker {
    choices: Vec<AgentChoice>,
    cursor: usize,
    open: bool,
    armed: Option<AgentChoice>,
}

impl MachinePicker {
    pub fn from_choices(choices: Vec<AgentChoice>) -> Option<Self> {
        if choices.is_empty() {
            None
        } else {
            Some(Self {
                choices,
                cursor: 0,
                open: false,
                armed: None,
            })
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.choices.iter().map(|choice| choice.name.as_str())
    }

    pub fn selected(&self) -> &AgentChoice {
        self.choice_at(self.cursor)
    }

    pub fn default_choice(&self) -> &AgentChoice {
        self.choice_at(0)
    }

    fn choice_at(&self, index: usize) -> &AgentChoice {
        self.choices
            .get(index)
            .or_else(|| self.choices.first())
            .expect("machine picker has a choice")
    }

    pub fn open(&mut self) {
        self.cursor = 0;
        self.open = true;
    }

    /// Rows the bordered picker block needs when painted inline (minimal mode): a border row,
    /// the "Machine" header, one row per choice, the key hint, and the closing border.
    pub fn inline_rows(&self) -> u16 {
        self.choices.len() as u16 + 4
    }

    pub fn cancel(&mut self) {
        self.open = false;
        self.armed = None;
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.choices.is_empty() {
            return;
        }
        let len = self.choices.len() as isize;
        let next = (self.cursor as isize + delta).rem_euclid(len) as usize;
        self.cursor = next;
    }

    pub fn confirm(&mut self) -> AgentChoice {
        let choice = self.selected().clone();
        self.open = false;
        self.armed = Some(choice.clone());
        choice
    }

    /// Decide whether this new-session request should ask, or which machine it uses.
    /// A confirmed choice is consumed so the next request asks again.
    pub fn begin_new_session(&mut self) -> NewSessionPick {
        if self.choices.is_empty() {
            return NewSessionPick::Skip;
        }
        if self.choices.len() == 1 {
            return NewSessionPick::Use(self.choices[0].clone());
        }
        if let Some(choice) = self.armed.take() {
            return NewSessionPick::Use(choice);
        }
        self.open();
        NewSessionPick::Ask
    }
}

/// Top-level config key: `[[agent_choice]]` with `name` and `cmd` strings.
pub const CONFIG_KEY: &str = "agent_choice";

/// A machine name is one word: ASCII letters, digits, `.`, `_`, `-`.
fn is_machine_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

/// `NAME=CMD`. The command may contain `=` and spaces. The name may not.
pub fn parse_agent_choice(raw: &str) -> Result<AgentChoice, String> {
    let (name, cmd) = raw
        .split_once('=')
        .ok_or_else(|| format!("--agent-choice expects NAME=CMD, got {raw:?}"))?;
    let name = name.trim();
    let cmd = cmd.trim();
    if name.is_empty() || cmd.is_empty() {
        return Err(format!("--agent-choice expects NAME=CMD, got {raw:?}"));
    }
    if !is_machine_name(name) {
        return Err(format!("--agent-choice name {name:?} is not a machine name"));
    }
    Ok(AgentChoice {
        name: name.to_string(),
        cmd: cmd.to_string(),
    })
}

pub fn parse_agent_choices(raw: &[String]) -> Result<Vec<AgentChoice>, String> {
    raw.iter().map(|spec| parse_agent_choice(spec)).collect()
}

/// `[[agent_choice]]` tables of an effective config, in file order. No key is no choices.
///
/// Each table needs a non-empty `name` (a machine name, as on the CLI) and a non-empty `cmd`.
/// A table that is not shaped like that is an error naming the table index, never skipped: a
/// user who wrote a picker entry should not silently get no picker.
pub fn agent_choices_from_config(root: &toml::Value) -> Result<Vec<AgentChoice>, String> {
    let Some(value) = root.get(CONFIG_KEY) else {
        return Ok(Vec::new());
    };
    let tables = value.as_array().ok_or_else(|| {
        format!("config.toml: `{CONFIG_KEY}` must be an array of tables ([[{CONFIG_KEY}]])")
    })?;
    tables
        .iter()
        .enumerate()
        .map(|(index, table)| {
            let field = |key: &str| -> Result<String, String> {
                let text = table
                    .get(key)
                    .ok_or_else(|| {
                        format!("config.toml: [[{CONFIG_KEY}]] #{} is missing `{key}`", index + 1)
                    })?
                    .as_str()
                    .ok_or_else(|| {
                        format!("config.toml: [[{CONFIG_KEY}]] #{} `{key}` must be a string", index + 1)
                    })?
                    .trim();
                if text.is_empty() {
                    return Err(format!(
                        "config.toml: [[{CONFIG_KEY}]] #{} `{key}` is empty",
                        index + 1
                    ));
                }
                Ok(text.to_string())
            };
            let name = field("name")?;
            let cmd = field("cmd")?;
            if !is_machine_name(&name) {
                return Err(format!(
                    "config.toml: [[{CONFIG_KEY}]] #{} name {name:?} is not a machine name",
                    index + 1
                ));
            }
            Ok(AgentChoice { name, cmd })
        })
        .collect()
}

/// `[[agent_choice]]` from the effective config on disk (user config plus the managed layers).
/// An unreadable config is no choices; a malformed `[[agent_choice]]` table is an error.
pub fn load_config_agent_choices() -> Result<Vec<AgentChoice>, String> {
    match xai_grok_shell::config::load_effective_config() {
        Ok(root) => agent_choices_from_config(&root),
        Err(err) => {
            tracing::warn!(error = %err, "agent_choice: effective config unreadable; no config choices");
            Ok(Vec::new())
        }
    }
}

/// The one list the pager offers. `--agent-choice` flags, when present, replace the config
/// list entirely; the two are never merged.
pub fn resolve_agent_choices(
    cli: &[String],
    config: &[AgentChoice],
) -> Result<Vec<AgentChoice>, String> {
    if cli.is_empty() {
        Ok(config.to_vec())
    } else {
        parse_agent_choices(cli)
    }
}

/// Header / roster label. The machine stays visible once the session has its own title.
pub fn header_label(machine: Option<&str>, title: Option<&str>) -> Option<String> {
    let machine = machine.map(str::trim).filter(|text| !text.is_empty());
    let title = title.map(str::trim).filter(|text| !text.is_empty());
    match (machine, title) {
        (Some(machine), Some(title)) => Some(format!("{machine} · {title}")),
        (Some(machine), None) => Some(machine.to_string()),
        (None, Some(title)) => Some(title.to_string()),
        (None, None) => None,
    }
}

pub fn handle_picker_key(picker: &mut MachinePicker, key: &KeyEvent) -> PickerKey {
    match key.code {
        KeyCode::Up | KeyCode::Char('k') => {
            picker.move_cursor(-1);
            PickerKey::Moved
        }
        KeyCode::Down | KeyCode::Char('j') => {
            picker.move_cursor(1);
            PickerKey::Moved
        }
        KeyCode::Enter => {
            picker.confirm();
            PickerKey::Confirmed
        }
        KeyCode::Esc => {
            picker.cancel();
            PickerKey::Cancelled
        }
        _ => PickerKey::Ignored,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerKey {
    Moved,
    Confirmed,
    Cancelled,
    Ignored,
}

/// Widest the picker block ever gets; both paint paths share it so the box looks the same in every mode.
const PICKER_WIDTH: u16 = 36;

/// Centered popup over the fullscreen / inline TUI. Clears the cells under the box first.
pub fn paint_picker(area: Rect, buf: &mut Buffer, picker: &MachinePicker) {
    let width = PICKER_WIDTH.min(area.width.saturating_sub(4)).max(20);
    let height = picker.inline_rows().min(area.height.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height: height.max(4),
    };
    Clear.render(rect, buf);
    paint_picker_block(rect, buf, picker);
}

/// Flush-left block for the minimal (scrollback-native) live region, which has no popup layer: the box is painted
/// into `area` top-left, in place of the prompt, so the choices sit right above the status line. Nothing outside
/// the box is touched; the caller owns clearing.
pub fn paint_picker_inline(area: Rect, buf: &mut Buffer, picker: &MachinePicker) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let rect = Rect {
        x: area.x,
        y: area.y,
        width: PICKER_WIDTH.min(area.width),
        height: picker.inline_rows().min(area.height),
    };
    paint_picker_block(rect, buf, picker);
}

fn paint_picker_block(rect: Rect, buf: &mut Buffer, picker: &MachinePicker) {
    let mut lines = vec![Line::from(Span::styled(
        "Machine",
        Style::default().add_modifier(Modifier::BOLD),
    ))];
    for (idx, choice) in picker.choices.iter().enumerate() {
        let marker = if idx == picker.cursor { "> " } else { "  " };
        let style = if idx == picker.cursor {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(Span::styled(
            format!("{marker}{}", choice.name),
            style,
        )));
    }
    lines.push(Line::from("enter select · esc cancel"));
    Paragraph::new(lines)
        .block(Block::default().borders(Borders::ALL).title(" New session "))
        .render(rect, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, KeyEventKind, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::empty())
    }

    #[test]
    fn parse_agent_choice_splits_name_and_command() {
        let choice = parse_agent_choice("khoj=ssh -T sab@khoj /opt/worker").unwrap();
        assert_eq!(choice.name, "khoj");
        assert_eq!(choice.cmd, "ssh -T sab@khoj /opt/worker");
    }

    #[test]
    fn parse_agent_choice_keeps_equals_inside_the_command() {
        let choice = parse_agent_choice("mini=env FOO=bar grok agent stdio").unwrap();
        assert_eq!(choice.name, "mini");
        assert_eq!(choice.cmd, "env FOO=bar grok agent stdio");
    }

    #[test]
    fn parse_agent_choice_rejects_a_missing_command() {
        assert!(parse_agent_choice("mini").is_err());
        assert!(parse_agent_choice("mini=").is_err());
        assert!(parse_agent_choice("=grok").is_err());
        assert!(parse_agent_choice("has space=grok").is_err());
    }

    #[test]
    fn repeated_choices_keep_order_and_default_to_the_first() {
        let choices = parse_agent_choices(&[
            "mini=grok-mini".into(),
            "khoj=grok-khoj".into(),
        ])
        .unwrap();
        let picker = MachinePicker::from_choices(choices).unwrap();
        assert_eq!(picker.default_choice().name, "mini");
        assert_eq!(picker.selected().name, "mini");
        assert!(!picker.is_open());
    }

    #[test]
    fn two_choices_ask_before_a_new_session_and_confirm_uses_the_highlighted_name() {
        let mut picker = MachinePicker::from_choices(parse_agent_choices(&[
            "mini=grok-mini".into(),
            "khoj=grok-khoj".into(),
        ])
        .unwrap())
        .unwrap();
        assert_eq!(picker.begin_new_session(), NewSessionPick::Ask);
        assert!(picker.is_open());
        assert_eq!(picker.selected().name, "mini");
        assert_eq!(handle_picker_key(&mut picker, &key(KeyCode::Down)), PickerKey::Moved);
        assert_eq!(picker.selected().name, "khoj");
        assert_eq!(
            handle_picker_key(&mut picker, &key(KeyCode::Enter)),
            PickerKey::Confirmed
        );
        assert!(!picker.is_open());
        assert_eq!(
            picker.begin_new_session(),
            NewSessionPick::Use(AgentChoice {
                name: "khoj".into(),
                cmd: "grok-khoj".into(),
            })
        );
        assert_eq!(picker.begin_new_session(), NewSessionPick::Ask);
    }

    #[test]
    fn one_choice_is_used_without_asking_and_cancel_does_not_arm() {
        let mut single = MachinePicker::from_choices(parse_agent_choices(&["mini=grok-mini".into()]).unwrap())
            .unwrap();
        assert_eq!(
            single.begin_new_session(),
            NewSessionPick::Use(AgentChoice {
                name: "mini".into(),
                cmd: "grok-mini".into(),
            })
        );
        let mut picker = MachinePicker::from_choices(parse_agent_choices(&[
            "mini=grok-mini".into(),
            "khoj=grok-khoj".into(),
        ])
        .unwrap())
        .unwrap();
        picker.open();
        assert_eq!(handle_picker_key(&mut picker, &key(KeyCode::Esc)), PickerKey::Cancelled);
        assert!(!picker.is_open());
        assert_eq!(picker.begin_new_session(), NewSessionPick::Ask);
    }

    #[test]
    fn header_label_keeps_the_machine_beside_the_session_title() {
        assert_eq!(header_label(Some("khoj"), None).as_deref(), Some("khoj"));
        assert_eq!(
            header_label(Some("khoj"), Some("Pong Game")).as_deref(),
            Some("khoj · Pong Game")
        );
        assert_eq!(header_label(None, Some("Pong Game")).as_deref(), Some("Pong Game"));
        assert_eq!(header_label(None, None), None);
    }

    #[test]
    fn empty_choice_list_is_no_picker() {
        assert!(MachinePicker::from_choices(vec![]).is_none());
    }

    fn buffer_text(buf: &Buffer) -> Vec<String> {
        let area = buf.area;
        (area.y..area.y + area.height)
            .map(|y| {
                (area.x..area.x + area.width)
                    .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                    .collect::<String>()
            })
            .collect()
    }

    /// Minimal mode has no popup layer, so the picker paints flush-left into the row band it was given.
    /// The cursor marker and both names must be visible, and nothing outside the box may be touched.
    #[test]
    fn inline_picker_paints_flush_left_with_every_choice_and_the_cursor() {
        let mut picker = MachinePicker::from_choices(parse_agent_choices(&[
            "claude=claude-agent-acp".into(),
            "pi=pi-acp".into(),
        ])
        .unwrap())
        .unwrap();
        assert_eq!(picker.begin_new_session(), NewSessionPick::Ask);
        assert_eq!(picker.inline_rows(), 6);

        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        paint_picker_inline(area, &mut buf, &picker);
        let rows = buffer_text(&buf);
        let text = rows.join("\n");
        assert!(text.contains("New session"), "title must paint:\n{text}");
        assert!(text.contains("Machine"), "header must paint:\n{text}");
        assert!(text.contains("> claude"), "cursor row must paint:\n{text}");
        assert!(text.contains("  pi"), "second choice must paint:\n{text}");
        assert!(text.contains("enter select · esc cancel"), "hint must paint:\n{text}");
        // Flush-left: the border starts in column 0, the cursor row starts just inside it.
        assert!(rows[0].starts_with('┌'), "top border at column 0:\n{text}");
        assert!(rows[2].starts_with("│> claude"), "cursor row flush-left:\n{text}");
        // Nothing past the 36-column box is painted.
        assert!(
            rows.iter()
                .all(|row| row.chars().skip(36).collect::<String>().trim().is_empty()),
            "cells right of the box stay untouched:\n{text}"
        );

        handle_picker_key(&mut picker, &key(KeyCode::Down));
        let mut buf = Buffer::empty(area);
        paint_picker_inline(area, &mut buf, &picker);
        let text = buffer_text(&buf).join("\n");
        assert!(text.contains("  claude") && text.contains("> pi"), "cursor moved to pi:\n{text}");

        // A band too short for the whole box still paints the top rows instead of panicking.
        let short = Rect::new(0, 0, 80, 3);
        let mut buf = Buffer::empty(short);
        paint_picker_inline(short, &mut buf, &picker);
        assert!(buffer_text(&buf).join("\n").contains("New session"));
    }

    fn config(text: &str) -> toml::Value {
        toml::from_str(text).unwrap()
    }

    #[test]
    fn config_tables_parse_in_file_order_and_the_first_is_the_default() {
        let root = config(
            r#"
[ui]
theme = "dark"

[[agent_choice]]
name = "claude"
cmd = "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/x/claude /x/claude-agent-acp"

[[agent_choice]]
name = "pi"
cmd = "/usr/bin/pi-acp"
"#,
        );
        let choices = agent_choices_from_config(&root).unwrap();
        assert_eq!(
            choices,
            vec![
                AgentChoice {
                    name: "claude".into(),
                    cmd: "env -u CLAUDECODE CLAUDE_CODE_EXECUTABLE=/x/claude /x/claude-agent-acp".into(),
                },
                AgentChoice {
                    name: "pi".into(),
                    cmd: "/usr/bin/pi-acp".into(),
                },
            ]
        );
        let mut picker = MachinePicker::from_choices(choices).unwrap();
        assert_eq!(picker.default_choice().name, "claude");
        assert_eq!(picker.begin_new_session(), NewSessionPick::Ask);
    }

    #[test]
    fn config_without_the_key_is_no_picker() {
        let root = config("[ui]\ntheme = \"dark\"\n");
        let choices = agent_choices_from_config(&root).unwrap();
        assert!(choices.is_empty());
        assert!(MachinePicker::from_choices(choices).is_none());
        let resolved = resolve_agent_choices(&[], &[]).unwrap();
        assert!(MachinePicker::from_choices(resolved).is_none());
    }

    #[test]
    fn malformed_config_tables_are_errors_not_silence() {
        let missing_cmd = config("[[agent_choice]]\nname = \"pi\"\n");
        assert!(agent_choices_from_config(&missing_cmd).unwrap_err().contains("missing `cmd`"));
        let empty_name = config("[[agent_choice]]\nname = \"  \"\ncmd = \"pi-acp\"\n");
        assert!(agent_choices_from_config(&empty_name).unwrap_err().contains("`name` is empty"));
        let bad_name = config("[[agent_choice]]\nname = \"has space\"\ncmd = \"pi-acp\"\n");
        assert!(agent_choices_from_config(&bad_name).unwrap_err().contains("not a machine name"));
        let not_a_string = config("[[agent_choice]]\nname = \"pi\"\ncmd = 3\n");
        assert!(agent_choices_from_config(&not_a_string).unwrap_err().contains("must be a string"));
        let not_an_array = config("[agent_choice]\nname = \"pi\"\ncmd = \"pi-acp\"\n");
        assert!(agent_choices_from_config(&not_an_array).unwrap_err().contains("array of tables"));
    }

    #[test]
    fn cli_choices_replace_the_config_list_entirely() {
        let from_config = vec![
            AgentChoice { name: "claude".into(), cmd: "claude-agent-acp".into() },
            AgentChoice { name: "pi".into(), cmd: "pi-acp".into() },
        ];
        let cli = vec!["khoj=ssh khoj worker".to_string()];
        let resolved = resolve_agent_choices(&cli, &from_config).unwrap();
        assert_eq!(
            resolved,
            vec![AgentChoice { name: "khoj".into(), cmd: "ssh khoj worker".into() }]
        );
        let picker = MachinePicker::from_choices(resolved).unwrap();
        assert_eq!(picker.default_choice().name, "khoj");
        assert_eq!(picker.names().collect::<Vec<_>>(), vec!["khoj"]);
    }

    #[test]
    fn no_cli_choices_fall_back_to_config_in_order() {
        let from_config = vec![
            AgentChoice { name: "claude".into(), cmd: "claude-agent-acp".into() },
            AgentChoice { name: "pi".into(), cmd: "pi-acp".into() },
        ];
        let resolved = resolve_agent_choices(&[], &from_config).unwrap();
        assert_eq!(resolved, from_config);
        let picker = MachinePicker::from_choices(resolved).unwrap();
        assert_eq!(picker.names().collect::<Vec<_>>(), vec!["claude", "pi"]);
    }

    #[test]
    fn a_bad_cli_choice_is_still_an_error_even_with_good_config() {
        let from_config = vec![AgentChoice { name: "pi".into(), cmd: "pi-acp".into() }];
        assert!(resolve_agent_choices(&["nocmd".to_string()], &from_config).is_err());
    }

    #[test]
    fn picker_key_ignores_release_events_only_by_code() {
        let _ = KeyEventKind::Release;
        let mut picker = MachinePicker::from_choices(parse_agent_choices(&[
            "mini=a".into(),
            "khoj=b".into(),
        ])
        .unwrap())
        .unwrap();
        picker.open();
        assert_eq!(handle_picker_key(&mut picker, &key(KeyCode::Char('x'))), PickerKey::Ignored);
        assert_eq!(picker.selected().name, "mini");
    }
}
