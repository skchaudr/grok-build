//! Named agent commands (`--agent-choice NAME=CMD`) and the new-session machine picker.
//!
//! One pager can offer several machines. The first choice is the default. With two or more,
//! creating a session asks which name to use before `session/new` is sent.

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
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
    {
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

pub fn paint_picker(area: Rect, buf: &mut Buffer, picker: &MachinePicker) {
    let width = 36.min(area.width.saturating_sub(4)).max(20);
    let height = (picker.choices.len() as u16 + 4).min(area.height.saturating_sub(2));
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let rect = Rect {
        x,
        y,
        width,
        height: height.max(4),
    };
    Clear.render(rect, buf);
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
