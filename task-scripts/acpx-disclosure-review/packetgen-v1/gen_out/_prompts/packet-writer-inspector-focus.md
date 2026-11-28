Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. Treat the attached excerpts as evidence;
lines outside the excerpts are not available to you. Do not invent function
names, line numbers or files that the repository map and excerpts do not show.

=== REPOSITORY CONTEXT (13 excerpts) ===

### `task-scripts/acpx-disclosure-review/packetgen-v1/_bundle/objective.md` (repository grok-build, complete file)
````
# Objective: packet 7, inspector body cursor and range

You are writing the task packet (not the implementation) for the packet below. Facts as of 9f61d30d on main. Packet 3b (detail-pane focus and linewise copy) has landed in 92f80438, was used on the installed build, and copied exact lines, wrapped lines and ranges. Its implementation is in views/acpx_modal/detail.rs and is the pattern packet 7 reuses.

## The packet as the operator wrote it
### 7. Inspector body

The inspector body gets the same cursor and range. Its Tab and Shift+Tab keep switching inspector tabs; `Y` copies the whole tab, which is what `y` does today, and `y` copies the cursor line or range.

## The shared key contract (verbatim, from the focus and copy track)
From the friction logs of 11-26 and 11-27: the modal shows evidence that can't be reached, selected or copied. Three packets share one key contract, taken from the pager's line viewer, so every read-only pane behaves the same:

- Tab and Shift+Tab move focus between the panes of the current view, both directions; the focused pane is marked the way existing views mark focus.
- In a focused text pane: `j`/`k` and Up/Down move a line cursor and keep it visible; `v` starts or ends a visual line range; `y` copies the cursor line or the range; `Y` copies the whole pane.
- Copy is plain text: no styles, no indentation or breaks added by wrapping; a wrapped source line copies whole. The notice reads as a normal copy message.
- Esc first ends visual mode, then returns focus to where the operator came from, then leaves the view, landing on the same row or stage they left.
- Keys on a list or tree keep their current meaning when it has focus.

## Coordinator integration item (verbatim)
- **Disjoint edits in `acpx_modal.rs`.** Packet 2 edits `row_extra` and its tests. Packet 4 edits modal preview/input/render and `LEDGER_KEYS`, including the group Space help. Packet 3b owns Run-mode focus and `RUN_KEYS`; refresh its excerpts after the opening changes land. Match each OLD block by content and check it does not overlap another packet's.

## Defects the operator hit on the installed build (sab-air, 2026-11-27 23:29), in his words
He opened /acpx, a past run, one step with Enter (the inspector), and copied output. The page looks good and works badly:
1. To read another step he has to press Esc, go down a step and press Enter again; he wants to open one step and use the arrow keys to reach the others.
2. Tab on a step moves between the step's output and its meta (inspector tabs). That is expected; it stays.
3. y copies the whole tab, one line or fifty. He wants the same linewise copy the grok build line viewer has.

## What the code shows about those defects
Step switching already exists as [ and ] (STEP_KEYS: Previous / next step in run), listed only in the ? overlay, and Left/Right are bound to horizontal scroll (also h/l). Up/Down and j/k scroll today and become the line cursor under the contract, so they cannot also change steps. Today y copies the whole active tab (test copy_returns_active_tab_full_text); after this packet Y does that and y copies the cursor line or visual range.

## Decision made for the writer (the coordinator's pick; the operator may overrule)
Left and Right switch to the previous and next step, like [ and ]; h and l keep horizontal scroll. The footer of the inspector shows the step keys and the cursor keys while it is open.

````

### `task-scripts/acpx-disclosure-review/packetgen-v1/_bundle/repo-map.md` (repository grok-build, complete file)
````
# Repository map (line numbers at HEAD 9f61d30d)

File sizes in lines:
  acpx_modal.rs
  acpx_modal/inspector.rs
  acpx_modal/detail.rs
    6957 total

## views/acpx_modal/inspector.rs: items (line: text)
  17:const ARTIFACT_MAX_BYTES: usize = 256 * 1024;
  18:const HORIZ_SCROLL_STEP: usize = 8;
  21:pub enum InspectorOutcome {
  32:enum TabKind {
  42:impl TabKind {
  43:    fn label(self) -> &'static str {
  57:struct TabEntry {
  67:pub struct InspectorState {
  78:impl InspectorState {
  80:    pub fn new(
  224:    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome {
  380:    pub fn scroll(&mut self, delta: isize) -> bool {
  399:    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, _now_ms: i64) {
  425:    pub fn shortcuts(&self) -> Vec<&'static str> {
  440:    fn max_scroll_y(&self) -> usize {
  451:    fn compute_visual_lines(
  490:    fn render_header(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
  554:    fn render_tabs(&self, buf: &mut Buffer, row: Rect, theme: &Theme) {
  581:    fn render_body(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
  682:struct VisualLine {
  688:fn widest_source_line(text: &str) -> usize {
  695:fn centre_offset(block_w: usize, block_h: usize, width: usize, height: usize) -> (usize, usize) {
  702:fn outcome_color(outcome: Outcome, theme: &Theme) -> Color {
  711:fn read_artifact_tab(run_dir: &Path, artifact: &ArtifactRef) -> (String, bool) {
  727:fn build_meta_text(step: &Step, node: Option<&FlowNode>) -> String {
  816:fn fmt_utc_hms_ms(ms: i64) -> String {
  823:fn decimal_digits(mut n: usize) -> usize {
  835:fn wrap_line_by_width(line: &str, max_width: usize) -> Vec<String> {
  863:fn slice_from_col(line: &str, skip_cols: usize) -> String {
  885:#[cfg(test)]
  886:mod tests {
  892:    fn sample_shell_step() -> Step {
  928:    fn sample_acp_step() -> Step {
  953:    fn row_text(buf: &Buffer, y: u16) -> String {
  962:    fn shell_action_tab_set_and_empty_stderr() {
  995:    fn acp_step_tab_set() {
  1011:    fn failed_step_opens_on_error_tab() {
  1022:    fn digit_and_tab_cycle_selection() {
  1054:    fn per_tab_scroll_memory_and_clamping() {
  1100:    fn wrap_toggle_changes_visual_line_count() {
  1118:    fn wrapping_wide_cjk_chars_never_exceeds_width() {
  1135:    fn copy_returns_active_tab_full_text() {
  1144:    fn json_output_tab_reads_as_rows_and_copy_still_gives_raw_json() {
  1173:    fn string_output_tab_is_unchanged() {
  1187:    fn replay_only_on_step_with_conversation() {
  1203:    fn prev_next_and_back_keys() {
  1221:    fn missing_artifact_shows_error_text_in_tab() {
  1237:    fn render_100x20_has_header_and_tabs_and_0x0_does_not_panic() {
  1267:    fn render_text(text: &str, wrap: bool, w: u16, h: u16, keys: &[char]) -> Buffer {
  1286:    fn first_content_cell(buf: &Buffer) -> Option<(u16, u16)> {
  1295:    fn short_content_is_centred_as_one_block() {
  1318:    fn tall_content_starts_at_the_top_row() {
  1333:    fn wide_content_starts_at_the_left_column() {
  1339:    fn wrap_mode_centres_the_wrapped_lines() {
  1355:    fn tiny_areas_do_not_panic() {

## views/acpx_modal.rs: items (line: text)
  1:pub(crate) mod detail;
  2:pub(crate) mod inspector;
  3:pub(crate) mod ledger;
  4:pub(crate) mod modal_view;
  5:pub(crate) mod run;
  34:const MAX_TITLE_WIDTH: usize = 72;
  36:const PREVIEW_MIN_WIDTH: u16 = 120;
  38:const EXTRAS_LIMIT: usize = 80;
  41:pub enum Mode {
  49:pub struct ReplayRequest {
  57:pub enum AcpxOutcome {
  67:struct ModalNotice {
  72:impl ModalNotice {
  73:    fn warning(text: String) -> Self {
  78:pub struct AcpxModalState {
  106:impl AcpxModalState {
  107:    pub fn open() -> Self {
  111:    pub fn open_at(runs_base: Option<PathBuf>, sessions_base: Option<PathBuf>) -> Self {
  136:    pub fn open_run(&mut self, dir: &Path) -> bool {
  160:    pub fn mode(&self) -> Mode {
  164:    pub fn wants_poll(&self) -> bool {
  180:    pub fn poll(&mut self) -> bool {
  200:    pub fn wants_esc(&self) -> bool {
  204:    pub fn notice(&self) -> Option<&str> {
  209:    pub fn set_notice(&mut self, text: String) {
  213:    pub fn report_copy(&mut self, delivery: &crate::clipboard::CopyDelivery) {
  222:    pub fn handle_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  264:    pub fn handle_mouse(&mut self, mouse: &MouseEvent) -> AcpxOutcome {
  312:    fn rescan_runs(&mut self) -> bool {
  326:    fn load_extras(&mut self, runs: &[RunSummary]) {
  355:    fn sync_preview(&mut self) {
  369:    fn rescan_sessions(&mut self) {
  378:    fn handle_ledger_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  398:    fn watch_open_mirror_run(&mut self, dir: &Path, bundle_run_id: &str) {
  415:    pub fn release_open_mirror_run(&mut self) {
  425:    pub fn cancel_open_mirror_run(&mut self) -> bool {
  452:    fn poll_answerers(&mut self) -> bool {
  479:    fn answer_mirror_gate(&mut self, run_id: &str, choice_label: &str, choice_key: &str) -> AcpxOutcome {
  505:    fn handle_run_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  634:    fn handle_step_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
  677:    fn open_step_at(&mut self, idx: usize) -> bool {
  700:    fn replay_step(&mut self, idx: usize) -> AcpxOutcome {
  726:    fn title_text(&self) -> String {
  750:    fn active_shortcut_labels(&self) -> Vec<&'static str> {
  776:fn row_extra(summary: &RunSummary) -> RowExtra {
  805:fn parse_gate_answer(input: &serde_json::Value) -> Option<GateAnswer> {
  822:fn read_run_answers(dir: &Path) -> Option<GateAnswer> {
  829:fn session_replay(s: SessionSummary) -> ReplayRequest {
  838:pub fn render_acpx_modal(
  948:fn render_notice_row(buf: &mut Buffer, content: Rect, notice: &ModalNotice, theme: &Theme) {
  969:const LEDGER_KEYS: &[(&str, &str)] = &[
  985:const RUN_KEYS: &[(&str, &str)] = &[
  1004:const STEP_KEYS: &[(&str, &str)] = &[
  1015:fn render_help_overlay(buf: &mut Buffer, area: Rect, mode: Mode, theme: &Theme) {
  1085:fn now_ms() -> i64 {
  1091:#[cfg(test)]
  1092:mod tests {
  1098:    fn key(code: KeyCode) -> KeyEvent {
  1102:    struct TestFixture {
  1111:    fn write_fixture(status: &str) -> TestFixture {
  1242:    fn buffer_text(buf: &Buffer) -> String {
  1255:    fn copy_notice_distinguishes_confirmed_clipboard_from_file_fallback() {
  1280:    fn open_at_lists_runs_and_sessions_and_navigates_modes() {
  1313:    fn wants_esc_reflects_mode_and_help_state() {
  1337:    fn replay_on_acp_step_returns_expected_request() {
  1362:    fn opening_session_in_ledger_returns_replay_with_none_range() {
  1383:    fn step_next_bracket_advances_and_esc_preserves_run_selection() {
  1413:    fn detail_focus_tab_backtab_and_without_detail_area() {
  1444:    fn detail_focus_range_copy_all_esc_unwind_and_tree_keys() {
  1515:    fn open_run_missing_dir_sets_notice_and_next_key_clears_it() {
  1534:    fn poll_detects_live_json_status_change_on_disk() {
  1567:    fn wants_poll_is_false_for_waiting_run() {
  1580:    fn question_mark_toggles_help_overlay_and_any_key_closes_it() {
  1604:    fn render_at_120x30_draws_title_and_tiny_sizes_do_not_panic() {
  1627:    fn row_extra_reads_gate_answer_and_handles_missing_or_malformed_gate() {
  1658:    fn run_outside_extras_limit_gets_answers_with_empty_rail() {
  1702:#[cfg(test)]
  1703:mod gate_answer_target_tests {
  1709:    fn key(code: KeyCode) -> KeyEvent {
  1715:    fn write_piw_gate_fixture() -> (tempfile::TempDir, PathBuf) {
  1771:    fn answer_target_is_parsed_and_routed_not_launched() {
  1808:    fn cancel_open_mirror_run_without_mirror_is_a_no_op() {

## views/acpx_modal/detail.rs: items (line: text)
  26:pub enum DetailTarget {
  33:struct CacheKey {
  45:struct CopyUnit {
  52:struct Rows {
  57:impl Rows {
  58:    fn new() -> Self {
  62:    fn push_unit(&mut self, rows: Vec<Line<'static>>, text: String) {
  72:    fn append_rows(&mut self, other: Rows) {
  82:    fn unit_at(&self, row: usize) -> Option<&CopyUnit> {
  89:    fn copy_range(&self, start_row: usize, end_row: usize) -> String {
  108:    fn copy_all(&self) -> String {
  117:impl std::ops::Deref for Rows {
  120:    fn deref(&self) -> &Self::Target {
  125:impl std::ops::DerefMut for Rows {
  126:    fn deref_mut(&mut self) -> &mut Self::Target {
  131:impl IntoIterator for Rows {
  135:    fn into_iter(self) -> Self::IntoIter {
  144:    fn into_iter(self) -> Self::IntoIter {
  149:fn line_plain_text(line: &Line<'_>) -> String {
  157:fn source_lines(text: &str) -> Vec<&str> {
  165:pub struct DetailPanel {
  180:impl Default for DetailPanel {
  181:    fn default() -> Self {
  186:impl DetailPanel {
  187:    pub fn new() -> Self {
  203:    pub fn set_answered(&mut self, answered: Option<&str>) {
  209:    pub fn is_focused(&self) -> bool {
  213:    pub fn is_visual(&self) -> bool {
  217:    pub fn set_focused(&mut self, focused: bool) {
  242:    pub fn page_step(&self) -> isize {
  252:    pub fn move_cursor(&mut self, delta: isize) -> bool {
  270:    pub fn toggle_visual(&mut self) -> bool {
  282:    pub fn handle_esc(&mut self) -> bool {
  294:    pub fn copy_selection(&self) -> Option<String> {
  299:    pub fn copy_all(&self) -> Option<String> {
  307:    fn selected_range(&self) -> Option<(usize, usize)> {
  322:    fn ensure_cursor_visible(&mut self, height: usize) {
  338:    pub fn scroll(&mut self, delta: isize) -> bool {
  360:    fn max_scroll(&self, height: usize) -> usize {
  368:    pub fn render(
  479:fn conversation_file_len(run: &Run, target: &DetailTarget) -> Option<u64> {
  503:fn build_lines(
  529:fn build_run_lines(
  719:fn step_one_line_summary(step: &Step) -> String {
  746:const EMPTY: &str = "(empty)";
  748:fn is_empty_container(val: &Value) -> bool {
  807:fn format_value_lines(val: &Value, width: usize, theme: &Theme) -> Rows {
  821:fn append_value_lines(
  986:fn first_non_empty_line(s: &str) -> String {
  994:fn build_step_lines(
  1013:fn build_step_lines_at(
  1314:fn failure_tail_for_step(run: &Run, step: &Step) -> Option<String> {
  1319:    const TAIL_ARTIFACT_MAX_BYTES: usize = 64 * 1024;
  1393:fn extract_step_note(step: &Step, verdict: Option<&StepVerdict>) -> Option<(String, bool)> {
  1394:    const NOTE_KEYS: &[&str] = &[
  1437:fn truncate_note_text(text: &str) -> (String, bool) {
  1438:    const MAX_LINES: usize = 3;
  1439:    const MAX_CHARS: usize = 240;
  1474:fn has_extra_result_fields(run: &Run, step: &Step, note_covered_entire_string: bool) -> bool {
  1494:fn is_digest_covered_key(key: &str, switch_on_key: Option<&str>) -> bool {
  1517:fn append_artifact_section(
  1525:    const MAX_BYTES: usize = 64 * 1024;
  1548:fn build_node_lines(
  1660:fn append_gate_section(
  1727:enum ConvItem {
  1738:fn append_conversation(
  1833:fn read_last_512k(path: &Path) -> std::io::Result<String> {
  1834:    const LIMIT: u64 = 512 * 1024;
  1853:fn parse_conversation_items(
  1997:fn format_trace_event(
  2020:fn push_section_rule(
  2045:fn title_row_right(
  2088:fn push_wrapped(
  2103:fn push_wrapped_capped(
  2134:fn dim_line(text: &str, theme: &Theme) -> Line<'static> {
  2141:fn truncate_line(line: Line<'static>, width: usize) -> Line<'static> {
  2161:fn run_status_color(status: RunStatus, theme: &Theme) -> Color {
  2171:fn node_state_color(state: NodeState, theme: &Theme) -> Color {
  2182:fn wrap(text: &str, width: usize) -> Vec<String> {
  2283:#[cfg(test)]
  2284:mod tests {
  2292:    fn make_test_run(dir: &Path) -> Run {
  2353:    fn buffer_lines(buf: &Buffer) -> Vec<String> {
  2367:    fn test_wrap_breaks_on_spaces_and_keeps_blank_lines() {
  2373:    fn test_wrap_hard_breaks_long_words_and_tabs() {
  2381:    fn test_wrap_wide_characters() {
  2388:    fn test_step_sections_order_prompt_result_events() {
  2440:    fn test_action_step_shows_command_exit_code_and_stdout() {
  2498:    fn test_conversation_thought_tool_call_and_message() {
  2567:    fn test_conversation_filters_seq_range_and_skips_auth_status_update() {
  2631:    fn test_missing_events_file_renders_dim_row() {
  2679:    fn test_run_target_lists_stages_and_waiting_gate_choices() {
  2749:    fn waiting_gate_run(dir: &Path) -> Run {
  2785:    fn panel_text(
  2799:    fn answered_gate_is_drawn_done_without_pickable_keys() {
  2820:    fn answered_gate_with_unknown_choice_ticks_nothing() {
  2831:    fn answer_arriving_redraws_a_cached_panel() {
  2850:    fn empty_list_and_object_values_read_as_empty_in_dim_style() {
  2908:    fn test_node_target_not_run() {
  2930:    fn test_scroll_clamps_and_returns_false_at_limits() {
  2975:    fn test_target_change_resets_scroll() {
  3022:    fn wrapped_failure_line_copies_exact_source_on_second_row() {
  3121:    fn focus_highlight_visual_range_and_target_change_reset_cursor() {
  3177:    fn test_failed_shell_orders_failure_tail_before_command_and_cwd_without_success() {
  3240:    fn test_repeated_attempt_shows_own_number_and_timing() {
  3299:    fn test_agent_with_brief_evidence_omits_prompt_wrappers_and_transcript_in_first_view() {
  3402:    fn test_agent_without_evidence_does_not_invent_it() {
  3438:    fn test_pending_gate_keeps_question_and_choices_conspicuous() {
  3458:    fn test_running_node_tail_follows_and_stops_on_scroll_up() {
  3570:    fn test_last_result_object_and_array_renders_clean_without_json_punctuation() {
  3635:    fn test_string_output_wraps_and_empty_or_null_hides_section() {
  3688:    fn test_array_output_of_40_items_renders_all_without_cap() {
  3723:    fn test_small_and_zero_rects_do_not_panic_and_stay_inside_area() {

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 17-78 of 1366 (repository grok-build)
````
const ARTIFACT_MAX_BYTES: usize = 256 * 1024;
const HORIZ_SCROLL_STEP: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub enum InspectorOutcome {
    Unchanged,
    Changed,
    Back,
    Prev,
    Next,
    Replay,
    Copy(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TabKind {
    Output,
    Prompt,
    Response,
    Stdout,
    Stderr,
    Error,
    Meta,
}

impl TabKind {
    fn label(self) -> &'static str {
        match self {
            Self::Output => "Output",
            Self::Prompt => "Prompt",
            Self::Response => "Response",
            Self::Stdout => "Stdout",
            Self::Stderr => "Stderr",
            Self::Error => "Error",
            Self::Meta => "Meta",
        }
    }
}

#[derive(Debug, Clone)]
struct TabEntry {
    kind: TabKind,
    text: String,
    is_empty_artifact: bool,
    scroll_y: usize,
    scroll_x: usize,
    /// What `y` copies when it differs from what the tab shows (a JSON value shown as rows copies its raw JSON).
    copy: Option<String>,
}

pub struct InspectorState {
    step: Step,
    node: Option<FlowNode>,
    position: (usize, usize),
    tabs: Vec<TabEntry>,
    active_tab: usize,
    wrap: bool,
    last_body_height: usize,
    last_body_width: usize,
}

impl InspectorState {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 224-440 of 1366 (repository grok-build)
````
    pub fn handle_key(&mut self, key: &KeyEvent) -> InspectorOutcome {
        if key.kind == KeyEventKind::Release {
            return InspectorOutcome::Unchanged;
        }

        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        match key.code {
            KeyCode::Esc => InspectorOutcome::Back,
            KeyCode::Tab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + 1) % self.tabs.len();
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::BackTab => {
                if !self.tabs.is_empty() {
                    self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char(c @ '1'..='7') if !ctrl => {
                let idx = (c as usize) - ('1' as usize);
                if idx < self.tabs.len() && idx != self.active_tab {
                    self.active_tab = idx;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('j') | KeyCode::Down if !ctrl => {
                if self.scroll(1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('k') | KeyCode::Up if !ctrl => {
                if self.scroll(-1) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageDown => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::PageUp => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('d') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('u') if ctrl => {
                let half = (self.last_body_height / 2).max(1) as isize;
                if self.scroll(-half) {
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('g') | KeyCode::Home if !ctrl => {
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y != 0
                {
                    tab.scroll_y = 0;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('G') | KeyCode::End if !ctrl => {
                let max_scroll = self.max_scroll_y();
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y != max_scroll
                {
                    tab.scroll_y = max_scroll;
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Left | KeyCode::Char('h') if !ctrl => {
                if self.wrap {
                    return InspectorOutcome::Unchanged;
                }
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_x > 0
                {
                    tab.scroll_x = tab.scroll_x.saturating_sub(HORIZ_SCROLL_STEP);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Right | KeyCode::Char('l') if !ctrl => {
                if self.wrap {
                    return InspectorOutcome::Unchanged;
                }
                if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                    tab.scroll_x = tab.scroll_x.saturating_add(HORIZ_SCROLL_STEP);
                    InspectorOutcome::Changed
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('w') if !ctrl => {
                self.wrap = !self.wrap;
                let max_scroll = self.max_scroll_y();
                if let Some(tab) = self.tabs.get_mut(self.active_tab)
                    && tab.scroll_y > max_scroll
                {
                    tab.scroll_y = max_scroll;
                }
                InspectorOutcome::Changed
            }
            KeyCode::Char('y') if !ctrl => {
                let text = self
                    .tabs
                    .get(self.active_tab)
                    .map(|t| t.copy.clone().unwrap_or_else(|| t.text.clone()))
                    .unwrap_or_default();
                InspectorOutcome::Copy(text)
            }
            KeyCode::Char('r') if !ctrl => {
                if self.step.conversation.is_some() {
                    InspectorOutcome::Replay
                } else {
                    InspectorOutcome::Unchanged
                }
            }
            KeyCode::Char('[') if !ctrl => InspectorOutcome::Prev,
            KeyCode::Char(']') if !ctrl => InspectorOutcome::Next,
            _ => InspectorOutcome::Unchanged,
        }
    }

    pub fn scroll(&mut self, delta: isize) -> bool {
        let max_scroll = self.max_scroll_y();
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        let cur = tab.scroll_y.min(max_scroll);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            (cur + delta as usize).min(max_scroll)
        };
        if next != tab.scroll_y {
            tab.scroll_y = next;
            true
        } else {
            false
        }
    }

    pub fn render(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme, _now_ms: i64) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Row 0: Header
        self.render_header(buf, Rect::new(area.x, area.y, area.width, 1), theme);

        if area.height < 2 {
            return;
        }

        // Row 1: Tabs
        self.render_tabs(buf, Rect::new(area.x, area.y + 1, area.width, 1), theme);

        if area.height < 3 {
            return;
        }

        // Rest: Body
        let body_area = Rect::new(area.x, area.y + 2, area.width, area.height - 2);
        self.last_body_height = body_area.height as usize;
        self.last_body_width = body_area.width as usize;
        self.render_body(buf, body_area, theme);
    }

    pub fn shortcuts(&self) -> Vec<&'static str> {
        let mut s = vec![
            "Tab/1-7: tab",
            "j/k: scroll",
            "w: wrap",
            "y: copy",
            "[/]: step",
        ];
        if self.step.conversation.is_some() {
            s.push("r: replay");
        }
        s.push("Esc: back");
        s
    }

    fn max_scroll_y(&self) -> usize {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/inspector.rs` lines 581-690 of 1366 (repository grok-build)
````
    fn render_body(&mut self, buf: &mut Buffer, area: Rect, theme: &Theme) {
        let width = area.width as usize;
        let height = area.height as usize;
        if width == 0 || height == 0 {
            return;
        }

        let Some(mut tab_snapshot) = self.tabs.get(self.active_tab).cloned() else {
            return;
        };

        if tab_snapshot.is_empty_artifact {
            let msg = "(empty)";
            let msg_cut = truncate_str(msg, width);
            let msg_w = msg_cut.width();
            let x = area.x + ((width.saturating_sub(msg_w)) / 2) as u16;
            let y = area.y;
            buf.set_stringn(x, y, &msg_cut, width, Style::default().fg(theme.gray_dim));
            return;
        }

        let total_src = tab_snapshot.text.lines().count().max(1);
        let digits = decimal_digits(total_src).max(2);
        let gutter_width = digits + 2;

        // A block that already fits the body does not scroll sideways.
        if !self.wrap && gutter_width + widest_source_line(&tab_snapshot.text) <= width {
            tab_snapshot.scroll_x = 0;
        }

        let (visual_lines, total_visual) = self.compute_visual_lines(&tab_snapshot, width);
        let max_scroll = total_visual.saturating_sub(height);
        let scroll_y = tab_snapshot.scroll_y.min(max_scroll);
        if let Some(tab_mut) = self.tabs.get_mut(self.active_tab) {
            tab_mut.scroll_y = scroll_y;
        }

        let widest_text = if self.wrap {
            visual_lines
                .iter()
                .map(|v| v.text.width())
                .max()
                .unwrap_or(0)
        } else {
            widest_source_line(&tab_snapshot.text)
        };
        let (x_off, y_off) = centre_offset(gutter_width + widest_text, total_visual, width, height);

        let gutter_style = Style::default().fg(theme.gray_dim);
        let text_style = if tab_snapshot.kind == TabKind::Error {
            Style::default().fg(theme.accent_error)
        } else {
            Style::default().fg(theme.text_primary)
        };

        for row_idx in 0..height {
            let v_idx = scroll_y + row_idx;
            let Some(vline) = visual_lines.get(v_idx) else {
                break;
            };
            let y = area.y + (y_off + row_idx) as u16;

            let gutter_str = match vline.line_no {
                Some(n) => format!("{n:>digits$}  "),
                None => " ".repeat(gutter_width),
            };

            if width <= gutter_width {
                let cut_g = truncate_str(&gutter_str, width);
                buf.set_stringn(area.x, y, &cut_g, width, gutter_style);
                continue;
            }

            let line = Line::from(vec![
                Span::styled(gutter_str, gutter_style),
                Span::styled(vline.text.clone(), text_style),
            ]);
            buf.set_line(area.x + x_off as u16, y, &line, (width - x_off) as u16);
        }

        // Bottom-right overflow counter `12-40/312`
        if total_visual > height {
            let start = scroll_y + 1;
            let end = (scroll_y + height).min(total_visual);
            let overlay = format!("{start}-{end}/{total_visual}");
            let ov_width = overlay.width();
            if ov_width <= width {
                let ox = area.x + (width - ov_width) as u16;
                let oy = area.y + (height - 1) as u16;
                buf.set_stringn(
                    ox,
                    oy,
                    &overlay,
                    ov_width,
                    Style::default().fg(theme.gray_dim).bg(theme.bg_base),
                );
            }
        }
    }
}

struct VisualLine {
    line_no: Option<usize>,
    text: String,
}

/// Display width of the widest source line.
fn widest_source_line(text: &str) -> usize {
    text.lines().map(|l| l.width()).max().unwrap_or(0)
}
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal/detail.rs` lines 40-330 of 3778 (repository grok-build)
````
    events_file_len: Option<u64>,
    answered: Option<String>,
}

#[derive(Debug, Clone)]
struct CopyUnit {
    start: usize,
    end: usize,
    text: String,
}

#[derive(Debug, Default)]
struct Rows {
    lines: Vec<Line<'static>>,
    units: Vec<CopyUnit>,
}

impl Rows {
    fn new() -> Self {
        Self::default()
    }

    fn push_unit(&mut self, rows: Vec<Line<'static>>, text: String) {
        if rows.is_empty() {
            return;
        }
        let start = self.lines.len();
        self.lines.extend(rows);
        let end = self.lines.len();
        self.units.push(CopyUnit { start, end, text });
    }

    fn append_rows(&mut self, other: Rows) {
        let offset = self.lines.len();
        self.lines.extend(other.lines);
        for mut unit in other.units {
            unit.start += offset;
            unit.end += offset;
            self.units.push(unit);
        }
    }

    fn unit_at(&self, row: usize) -> Option<&CopyUnit> {
        let idx = self.units.partition_point(|u| u.end <= row);
        self.units
            .get(idx)
            .filter(|u| u.start <= row && row < u.end)
    }

    fn copy_range(&self, start_row: usize, end_row: usize) -> String {
        if self.lines.is_empty() || start_row > end_row {
            return String::new();
        }
        let last = end_row.min(self.lines.len() - 1);
        let mut out = Vec::new();
        let mut row = start_row.min(last);
        while row <= last {
            if let Some(unit) = self.unit_at(row) {
                out.push(unit.text.clone());
                row = unit.end;
            } else {
                out.push(line_plain_text(&self.lines[row]));
                row += 1;
            }
        }
        out.join("\n")
    }

    fn copy_all(&self) -> String {
        if self.lines.is_empty() {
            String::new()
        } else {
            self.copy_range(0, self.lines.len() - 1)
        }
    }
}

impl std::ops::Deref for Rows {
    type Target = Vec<Line<'static>>;

    fn deref(&self) -> &Self::Target {
        &self.lines
    }
}

impl std::ops::DerefMut for Rows {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.lines
    }
}

impl IntoIterator for Rows {
    type Item = Line<'static>;
    type IntoIter = std::vec::IntoIter<Line<'static>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.into_iter()
    }
}

impl<'a> IntoIterator for &'a Rows {
    type Item = &'a Line<'static>;
    type IntoIter = std::slice::Iter<'a, Line<'static>>;

    fn into_iter(self) -> Self::IntoIter {
        self.lines.iter()
    }
}

fn line_plain_text(line: &Line<'_>) -> String {
    let mut text = String::new();
    for span in &line.spans {
        text.push_str(span.content.as_ref());
    }
    text.trim_end().to_string()
}

fn source_lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        vec![""]
    } else {
        text.lines().collect()
    }
}

pub struct DetailPanel {
    cache_key: Option<CacheKey>,
    /// The picked choice of an answered gate; `Some("")` when it was answered
    /// but the choice is unknown. `None` while the gate is open.
    answered: Option<String>,
    last_target: Option<(PathBuf, DetailTarget)>,
    lines: Rows,
    scroll: usize,
    tail_follow: bool,
    last_height: usize,
    focused: bool,
    cursor: usize,
    visual_anchor: Option<usize>,
}

impl Default for DetailPanel {
    fn default() -> Self {
        Self::new()
    }
}

impl DetailPanel {
    pub fn new() -> Self {
        Self {
            cache_key: None,
            answered: None,
            last_target: None,
            lines: Rows::new(),
            scroll: 0,
            tail_follow: false,
            last_height: 0,
            focused: false,
            cursor: 0,
            visual_anchor: None,
        }
    }

    /// Tell the panel whether the run's gate has been answered, and with what.
    pub fn set_answered(&mut self, answered: Option<&str>) {
        if self.answered.as_deref() != answered {
            self.answered = answered.map(str::to_string);
        }
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn is_visual(&self) -> bool {
        self.focused && self.visual_anchor.is_some()
    }

    pub fn set_focused(&mut self, focused: bool) {
        if self.focused == focused {
            return;
        }
        self.focused = focused;
        if !focused {
            self.visual_anchor = None;
            return;
        }
        self.tail_follow = false;
        if self.lines.is_empty() {
            self.cursor = 0;
            return;
        }
        let max_row = self.lines.len() - 1;
        self.cursor = self.cursor.min(max_row);
        if self.last_height > 0 {
            let top = self.scroll.min(max_row);
            let bottom = (top + self.last_height - 1).min(max_row);
            if self.cursor < top || self.cursor > bottom {
                self.cursor = top;
            }
        }
    }

    pub fn page_step(&self) -> isize {
        if self.last_height > 1 {
            (self.last_height - 1) as isize
        } else if self.last_height == 1 {
            1
        } else {
            15
        }
    }

    pub fn move_cursor(&mut self, delta: isize) -> bool {
        if self.lines.is_empty() || delta == 0 {
            return false;
        }
        let max_row = self.lines.len() - 1;
        let cur = self.cursor.min(max_row);
        let next = if delta < 0 {
            cur.saturating_sub(delta.unsigned_abs())
        } else {
            cur.saturating_add(delta as usize).min(max_row)
        };
        self.tail_follow = false;
        let moved = next != self.cursor;
        self.cursor = next;
        self.ensure_cursor_visible(self.last_height);
        moved
    }

    pub fn toggle_visual(&mut self) -> bool {
        if self.lines.is_empty() {
            return false;
        }
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
        } else {
            self.visual_anchor = Some(self.cursor.min(self.lines.len() - 1));
        }
        true
    }

    pub fn handle_esc(&mut self) -> bool {
        if !self.focused {
            return false;
        }
        if self.visual_anchor.is_some() {
            self.visual_anchor = None;
        } else {
            self.focused = false;
        }
        true
    }

    pub fn copy_selection(&self) -> Option<String> {
        let (start, end) = self.selected_range()?;
        Some(self.lines.copy_range(start, end))
    }

    pub fn copy_all(&self) -> Option<String> {
        if self.lines.is_empty() {
            None
        } else {
            Some(self.lines.copy_all())
        }
    }

    fn selected_range(&self) -> Option<(usize, usize)> {
        if !self.focused || self.lines.is_empty() {
            return None;
        }
        let max_row = self.lines.len() - 1;
        let cur = self.cursor.min(max_row);
        match self.visual_anchor {
            Some(anchor) => {
                let a = anchor.min(max_row);
                Some((a.min(cur), a.max(cur)))
            }
            None => Some((cur, cur)),
        }
    }

    fn ensure_cursor_visible(&mut self, height: usize) {
        if height == 0 || self.lines.is_empty() {
            return;
        }
        let max_row = self.lines.len() - 1;
        self.cursor = self.cursor.min(max_row);
        let max_scroll = self.max_scroll(height);
        self.scroll = self.scroll.min(max_scroll);
        if self.cursor < self.scroll {
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 634-725 of 1816 (repository grok-build)
````
    fn handle_step_key(&mut self, key: &KeyEvent) -> AcpxOutcome {
        let Some((step_idx, ins)) = self.inspector.as_mut() else {
            self.mode = Mode::Run;
            return AcpxOutcome::Changed;
        };
        let idx = *step_idx;
        match ins.handle_key(key) {
            InspectorOutcome::Unchanged => AcpxOutcome::Unchanged,
            InspectorOutcome::Changed => AcpxOutcome::Changed,
            InspectorOutcome::Back => {
                self.inspector = None;
                self.mode = Mode::Run;
                if let Some(rv) = self.run_view.as_mut() {
                    rv.select_step(idx);
                }
                AcpxOutcome::Changed
            }
            InspectorOutcome::Prev => {
                let target = idx.saturating_sub(1);
                if target != idx && self.open_step_at(target) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            InspectorOutcome::Next => {
                let total = self
                    .run_view
                    .as_ref()
                    .map_or(0, |rv| rv.run().steps.len());
                let max_idx = total.saturating_sub(1);
                let target = (idx + 1).min(max_idx);
                if target != idx && self.open_step_at(target) {
                    AcpxOutcome::Changed
                } else {
                    AcpxOutcome::Unchanged
                }
            }
            InspectorOutcome::Replay => self.replay_step(idx),
            InspectorOutcome::Copy(text) => AcpxOutcome::Copy(text),
        }
    }

    fn open_step_at(&mut self, idx: usize) -> bool {
        let Some(rv) = self.run_view.as_mut() else {
            return false;
        };
        let total = rv.run().steps.len();
        let Some(step) = rv.run().steps.get(idx).cloned() else {
            return false;
        };
        let node = rv
            .run()
            .flow
            .nodes
            .iter()
            .find(|n| n.id == step.node_id)
            .cloned();
        let run_dir = rv.run().summary.dir.clone();
        rv.select_step(idx);
        let inspector = InspectorState::new(&run_dir, step, node, (idx + 1, total));
        self.inspector = Some((idx, inspector));
        self.mode = Mode::Step;
        true
    }

    fn replay_step(&mut self, idx: usize) -> AcpxOutcome {
        let Some(rv) = self.run_view.as_ref() else {
            return AcpxOutcome::Unchanged;
        };
        let run = rv.run();
        let Some(step) = run.steps.get(idx) else {
            self.set_notice("no event log for this step".to_string());
            return AcpxOutcome::Changed;
        };
        let Some(conv) = step.conversation.as_ref() else {
            self.set_notice("no event log for this step".to_string());
            return AcpxOutcome::Changed;
        };
        let Some(events_path) = run.session_events_path(&conv.session_id) else {
            self.set_notice("no event log for this step".to_string());
            return AcpxOutcome::Changed;
        };
        let title = format!("{} › {}", run.summary.title, step.node_id);
        AcpxOutcome::Replay(ReplayRequest {
            events_path: events_path.to_path_buf(),
            range: Some((conv.event_start_seq, conv.event_end_seq)),
            session_id: conv.session_id.clone(),
            title,
        })
    }

````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 750-775 of 1816 (repository grok-build)
````
    fn active_shortcut_labels(&self) -> Vec<&'static str> {
        let mut labels = match self.mode {
            Mode::Ledger => self.ledger.shortcuts(),
            Mode::Run => {
                let captures = self.run_view.as_ref().is_some_and(|rv| rv.captures_keys());
                if self.detail.is_focused() && !captures {
                    vec!["j/k line", "v range", "y copy", "Y all", "Esc tree"]
                } else {
                    self.run_view
                        .as_ref()
                        .map_or_else(Vec::new, |rv| rv.shortcuts())
                }
            }
            Mode::Step => self
                .inspector
                .as_ref()
                .map_or_else(Vec::new, |(_, ins)| ins.shortcuts()),
        };
        if !labels.contains(&"? keys") {
            labels.push("? keys");
        }
        labels
    }
}

/// What the list shows for one run beyond its summary. Reads the run folder.
````

### `crates/codegen/xai-grok-pager/src/views/acpx_modal.rs` lines 1004-1014 of 1816 (repository grok-build)
````
const STEP_KEYS: &[(&str, &str)] = &[
    ("Tab / 1-5", "Switch inspector tab (Output / Prompt / Stdout / Stderr / Events)"),
    ("[ / ]", "Previous / next step in run"),
    ("j / k / ↑ / ↓", "Scroll active pane"),
    ("PgUp / PgDn", "Page up / down"),
    ("r", "Replay ACP conversation for this step"),
    ("y", "Copy current tab content"),
    ("?", "Toggle this key reference"),
    ("Esc / Backspace", "Back to run view"),
];

````

### `task-scripts/acpx-disclosure-slices.py` lines 14-90 of 575 (repository grok-build)
````
ROOT = Path(__file__).resolve().parents[1]
REPOS = {"grok": str(ROOT), "piw": str(Path.home() / "repos/pi-workflows")}
MODAL = "crates/codegen/xai-grok-pager/src/views/acpx_modal"
ACPX = "crates/codegen/xai-grok-pager/src/acpx"
APP_MODAL = f"{MODAL}.rs"
CHAT = "crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/agent.rs"
CHAT_DRIVER = "crates/codegen/xai-grok-pager-bin/src/bin/grok-acpx-acp/driver.rs"
GATE = "crates/codegen/xai-grok-pager/src/piw/gate.rs"
ARTIFACTS = "task-scripts/acpx-disclosure-returns"

SYSTEM = """Use only the supplied packet. Do not use tools, read or write files,
run commands, or dispatch other agents. Return exactly one file block with the
specified path and fence language. For edit tasks, return the Markdown edit
list in the format the task gives. For design tasks, return only the requested
Markdown artifact and use inline code or indented examples inside it. Do not
return placeholder code. Treat the attached excerpts as evidence; lines outside
the excerpts are not available to you. Rust edition 2024, toolchain 1.94.0. No
new dependencies, unsafe, or new production unwrap/expect. Preserve theme styles
and keymaps unless the task explicitly changes them. Do not change gate
authority, workflow state, or release behavior. Change ledger routing only
where the task explicitly owns a reviewed presentation or focus change."""

EDIT_FORMAT = """
Edit list format. The file is Markdown. List the edits in application order;
each edit has exactly this shape:

## Edit <N>: <short purpose>
Path: <repository-relative path>
OLD:
~~~rust
<existing lines copied verbatim from one attached excerpt>
~~~
NEW:
~~~rust
<complete replacement>
~~~

Rules:
- Copy OLD character for character from an attached excerpt, including
  indentation, blank lines and comments. Never retype, reformat, abbreviate or
  elide it. OLD must occur exactly once in the file: include enough lines to be
  unique, normally the whole function, impl item, struct or test.
- Every OLD quotes the original attached source, never the result of an
  earlier edit. No two OLD blocks overlap.
- To insert new code (a field, function or test), quote a nearby existing
  anchor as OLD and give the anchor plus the new code as NEW.
- NEW is complete code with no "...", "unchanged" or elided bodies.
- Prefer replacing whole functions and whole test functions to small fragments.
- Inside the file use only ~~~ fences, never triple backticks.
- Never edit code outside the attached excerpts. If the change needs that, add
  a final "## Coordinator follow-ups" section naming each location and reason.

The coordinator matches each OLD exactly once on the pinned base and stops on
zero or several matches, then applies the edits and runs format, compile and
the targeted tests.
"""


def source(path, *ranges, repo="grok"):
    return [f"{repo}:{path}#L{start}-{end}" for start, end in ranges]


def task(slice_id, context, spec, *, design=False):
    if design:
        extension, shape, tail = "md", "<complete Markdown artifact>", ""
    else:
        extension, shape, tail = "edits.md", "<complete Markdown edit list>", EDIT_FORMAT
    artifact = f"{ARTIFACTS}/{slice_id}.{extension}"
    return {
        "custom_id": slice_id,
        "context_files": context,
        "output_files": [f"grok:{artifact}"],
        "prompt": f"Slice ID: {slice_id}\n\n{spec.strip()}\n{tail}\n"
        f"Return exactly:\n=== FILE: {artifact} ===\n```markdown\n{shape}\n```",
    }


````

### `task-scripts/acpx-disclosure-slices.py` lines 236-292 of 575 (repository grok-build)
````
    task(
        "attempt-detail",
        source(f"{MODAL}/detail.rs", (1, 243), (436, 533), (680, 1102),
               (1370, 1649), (1650, 2035), (2104, 2377), (2548, 2639))
        + source(f"{MODAL}/inspector.rs", (1, 219), (320, 358))
        + source(f"{MODAL}/run.rs", (552, 611))
        + source(APP_MODAL, (795, 858))
        + source(f"{ACPX}/bundle.rs", (1, 350))
        + source(f"{ACPX}/step_digest.rs", (1, 85), (153, 259), (353, 460))
        + source(f"{ACPX}/step_evidence.rs", (32, 337), (339, 505)),
        rf"""
Ownership: the selected-attempt view in {MODAL}/detail.rs, including tests.
Edit no other file. Preserve the existing run and node views, inspector, ledger
rows, run copy behavior, gates, entry routing, and the Flows preview call in
{APP_MODAL} (attached read-only to show how DetailPanel is called). Keep
DetailTarget's public variants unchanged so the timeline slice can proceed
without touching its match arms. Preserve the existing scroll and narrow-width
behavior. Detail-pane focus and linewise copy are a later packet built on this
one; don't add a cursor or selection here.

Goal: show a compact attempt digest first, with the full evidence available
through current inspector, timeline expansion, and conversation replay.
For a failed shell attempt put the relevant bounded failure tail before the
command and cwd. Reuse step_digest, including effective outcome for a failed
shell attempt. Show attempt number, elapsed/timeout when available, and a
recorded exit code when available. Use the step position already provided by
the caller and the existing render time passed into build_lines; no invented
timeout, exit code, verdict or scope count. Preserve
any step/receipt settings that provide the actual context for the attempt.

For a finished agent step show a verdict or validator evidence when present,
a file-scope summary only when grounded in existing evidence
(digest_step_files with tools from read_attempt_tool_calls, called only inside
the cached build_lines path), and a short human-readable note if available. No
raw wrapper JSON or repeated transcript in the first view. Include accurate,
conditional directions for the current Enter inspector and r replay. Existing
details for a genuinely waiting gate must remain reachable and its
question/choices must stay conspicuous; leave its current answer routing and
labels untouched.

The digest goes first; the existing conversation, artifact and event sections
may follow it. Do not delete or rename any function: callers outside the
excerpts use them (append_conversation, parse_conversation_items,
read_last_512k and append_value_lines live outside the excerpts;
value_plain_text is used by the inspector and value_inline_summary by run.rs).
Keep meaningful evidence for other kinds of attempts available. Preserve
narrow-width wrapping and cache behavior. Don't add extra reads on every render.

Tests: failed shell orders tail before command and cwd without saying success
on a nonzero exit; repeated attempt shows its own number and timing; agent
with brief evidence does not dump its prompt/output wrappers/transcript in the
first view, and agent without evidence doesn't invent it; a pending gate still
presents the question and choices; full evidence affordances remain accurate.
Adapt tests that explicitly asserted the old detail order; keep unrelated
formatter and navigation expectations intact.
""",
    ),
````

### `task-scripts/acpx-disclosure-slices.py` lines 293-361 of 575 (repository grok-build)
````
    task(
        "bare-command-picker",
        source(CHAT, (1, 121), (1125, 1257), (1258, 1355), (1375, 1393), (1825, 2023),
               (2104, 2252), (2293, 2350), (2375, 2478), (2531, 2597), (2598, 2783))
        + source(CHAT_DRIVER, (394, 530), (1499, 1527), (2168, 2219)),
        rf"""
Ownership: {CHAT} and {CHAT_DRIVER}, including their tests. Edit nothing else.

Goal: in Grok ACPX chat a bare /open or bare /flow shows a pick list instead
of the help text. Today parse_command returns Command::Help for both; the
attached parse_command_unknown_slash_and_acpx_help test asserts that and must
change for these two commands only.

Bare /open: show one question through the existing x.ai/ask_user_question
machinery (ask_question_raw, shaped like ask_gate_question, noFreeform true)
listing the 20 most recent saved runs from list_saved_runs(&runs_dir,
runtime::mirror_root().as_deref(), 20), the same list /runs shows, newest
first, native ACPX and mirrored PIW runs together. Option id and label are the
run ID. The description carries what the operator needs to choose: flow name,
title when it differs from the flow name, source (SavedRunSource::label),
status, and start time formatted with the existing helper. Choosing a run does
exactly what `/open <run_id>` does now, through resolve_saved_run and
replay_run_into_session with the resolved source, so a mirrored run stays
read-only and a native run keeps its gate restore; move the current Open arm
into one helper that both paths call, keeping its "Already shown above." and
not-found replies. Dismissing the question, an unknown choice, or a cancelled
session opens nothing and sends one short line. No runs: reply
"No acpx runs found." without a question. Opening stays inspection only.

Bare /flow: list the configured default_flow() first when it exists, then
the flowPath values recorded in the manifests of the 20 most recent runs from
both sources, runs_dir() and runtime::mirror_root(), newest first, each with
the flow name, source, last status and last start time of its most recent
run. /flow <file>.workflow.ts already routes to PIW (is_workflow_file,
run_piw_flow), so a mirrored run's workflow is a valid candidate. Add one
driver function next to list_runs that reads one runs directory's manifests,
reusing read_json_file, json_str and parse_ts, and call it for each source;
skip manifests without a flowPath. Resolve every candidate, the default flow
included, the way explicit /flow resolves its file (resolve_flow_path against
session_cwd); skip a candidate whose resolved path is not an existing file
(recorded paths under /private/tmp are often gone); deduplicate by resolved
path, keeping the newest run's description. Selecting a candidate does exactly
what typing `/flow <resolved path>` does today: it goes through the existing
Flow arm with input {{}} and starts a new run, the .workflow.ts route included.
Move that arm's body into one helper both paths call. Dismissing the question,
an unknown choice or a cancelled session starts nothing and sends one short
line. No candidates left after filtering: reply with the help text as today.
Mirrored /open stays read-only.

Keep `/open <id>`, `/flow <file> ...`, /runs, /answer (bare /answer still
Help) and plain-text handling unchanged. Update available_commands and
help_markdown so both commands say the argument is optional. Keep the
question's toolCallId distinct from gate tool call IDs.

Tests: keep the attached saved_runs_list_both_sources_and_only_native_replay_restores_a_gate
test passing unchanged. parse_command returns the new bare variants for "/open" and "/flow"
(and still Help for "/runs extra" and bare "/answer"); build the option lists
through pure functions you add and test them: newest-first order across both
sources, run ID as id and label, description content including the source, title omitted when equal to the flow name,
flow candidates from both sources, a relative flowPath resolved against the
session cwd, a missing file skipped (pass the file check in as a closure so the
pure function needs no real paths), two runs whose paths resolve to the same
file collapsed to the newest, default flow first and not repeated, manifests
without flowPath skipped (driver test with temp dirs like the attached
test_list_runs_order_and_title_fallback), selection mapped to the same
Command::Flow {{ file, input: {{}} }} that typing /flow <path> produces, and the
empty cases.
""",
    ),
````

### `task-scripts/acpx-disclosure-slices.py` lines 489-573 of 575 (repository grok-build)
````
    task(
        "detail-focus",
        source(f"{MODAL}/detail.rs", (1, 260), (415, 445), (525, 740),
               (975, 995), (1220, 1275), (1365, 1380), (1436, 1500),
               (1725, 1835), (1975, 2058), (2477, 2540), (2622, 2713))
        + source(APP_MODAL, (1, 40), (78, 160), (199, 262), (377, 396),
                 (504, 558), (651, 694), (757, 864), (902, 917),
                 (1005, 1030), (1156, 1168), (1227, 1250), (1297, 1326))
        + source(f"{MODAL}/run.rs", (25, 45), (170, 215), (520, 545)),
        rf"""
Packet 3b. Ownership: {MODAL}/detail.rs (DetailPanel and its row building)
and, in {APP_MODAL}, only Run-mode routing: handle_key, handle_run_key,
wants_esc, open_run, active_shortcut_labels, the Mode::Run render branch,
RUN_KEYS, and their tests. Do not edit run.rs (attached read-only), the
ledger, the inspector, the Mode::Ledger preview branch or LEDGER_KEYS.

Goal, from the operator's friction log: in an open run he can Tab into the
detail pane of a failed stage, select the compiler-error line, copy it
exactly, and come back to the same stage.

Key contract, shared with later inspector and Agent Turns packets:
- Tab and Shift+Tab (KeyCode::Tab, KeyCode::BackTab) move focus between the
  stage tree and the detail pane, both directions. Only when the detail pane
  is drawn (RunViewState::detail_area() is Some) and the run view is not
  capturing keys (captures_keys()); otherwise Tab keeps doing nothing.
- With the detail pane focused: j/k and Up/Down move a line cursor by one row
  and keep it visible (scroll follows); PageUp/PageDown move by a page; v starts
  or ends a visual line range anchored at the cursor; y copies the cursor's
  copy unit, or every unit the range touches; Y copies the whole pane. Copies
  return AcpxOutcome::Copy(text), so the existing report_copy notice applies.
- Esc with the pane focused: first ends visual mode, then returns focus to the
  tree. Esc with the tree focused keeps leaving the run, as now.
- With the tree focused every current key keeps its meaning: v graph toggle,
  n noise toggle, y stage-output copy, J/K/D/U scrolling the detail pane, Space, Enter, r.
- With the pane focused, keys the pane does not use (r, [, ], F5, 1-9, c, !,
  n, Enter, ?) go to the run view exactly as before.
- Focus is shown by the cursor row: bg_highlight across the pane width, and
  the range rows in the same style while visual mode is on. Nothing is
  highlighted while the tree has focus. The footer shows the pane's keys
  while it has focus (e.g. "j/k line", "v range", "y copy", "Y all",
  "Esc tree"), and RUN_KEYS gains rows for Tab and the pane keys.
- Changing the selected stage (a new DetailTarget) moves the cursor to the
  first row and ends visual mode. Leaving the run (RunOutcome::Back) and
  open_run reset focus to the tree, so the Ledger preview never shows a
  cursor.

Copy units. The pane stores pre-wrapped rows, and wrap() drops the spaces it
breaks at, so copy text cannot be rebuilt from the rows. Record it where rows
are made: replace the row vector with a small module-private type (for
example `Rows`) that derefs to Vec<Line<'static>> so existing `out.push`,
`out.extend` and `.len()` keep working, and that also records, for rows
produced by wrapping one source line, the exact source line they copy as.
- Every row is in exactly one copy unit. A row pushed directly copies as its
  own visible text with trailing spaces trimmed. Rows from push_wrapped and
  push_wrapped_capped copy as the source line (each line of the text, split
  on newlines) whose wrapping produced them, with no indentation, prefix span
  or break added; a capped unit still copies its whole source line. The
  string-value loop in append_value_lines (the `wrap(s, body_w)` loop) does
  the same, copying the value's line without its indent span. Other direct
  wrap() loops may stay row-by-row.
- A range copies each touched unit once, in order, joined with newlines. Y
  copies every unit. Copies are plain text: no styles, no section-rule
  dashes or scrollbar characters added.
- Change the signatures of the helpers that take `&mut Vec<Line<'static>>`
  and of the builders that create the row vector, including
  format_value_lines, to the new type. Where rows built separately are added
  to the main rows (the `out.extend(...)` of value lines), keep their units,
  e.g. with an append method that offsets them; a plain Vec<Line> extended in
  copies row by row. No function is deleted or renamed; build_lines may
  return the new type.
- Cache: units are rebuilt with the rows under the existing CacheKey; no new
  reads, and nothing per frame beyond drawing.

Tests, in the existing styles: Tab then Shift+Tab move focus and back; Tab
does nothing without a detail area; with the pane focused j/k move the cursor
and v, j, y copy a two-unit range; y on the second row of a wrapped failure
line returns the whole source line exactly, e.g. a 70-character path that wraps
at width 30 copies as the original line with no added space or newline; Y
copies all units; Esc ends visual, then returns focus to the tree, then
leaves the run, landing on the same run in the list; with the tree focused y
still returns the stage output and v still toggles the graph; a new target resets
the cursor; the copied text contains no section-rule characters. Update the
wants_esc test only if its expectations change.
""",
    ),
````

=== TASK SPECIFICATION ===

Slice ID: packet-writer-inspector-focus

You write task packets for other one-shot workers. You do not implement the
task. A worker you never talk to will receive exactly: a fixed system note, the
source excerpts you list, and the task text you write. It has no tools and
cannot ask questions. The attached examples in task-scripts/acpx-disclosure-slices.py (attempt-detail,
bare-command-picker, detail-focus) are packets that were dispatched and landed;
match their shape. The objective for your packet is in objective.md. The
repository map gives every item with its line number.

Write the packet for packet 7 (inspector body cursor and range). The worker
will return an exact old/new edit list; the harness adds the edit-list format
and the return block to your task text automatically, so do not write them.

Your task text must contain, as the examples do:
- Ownership: the files, and the exact functions, structs, constants and tests the
  worker may edit; what it must not edit and why.
- Goal in the operator's terms, with the acceptance sentence.
- The behaviour contract as exact keys and states: what each key does with the
  body focused and unfocused, what Esc does at each step, what the footer and
  help show, and what must keep its current meaning.
- Representation guidance only where the code forces a choice. Say which landed
  3b types and methods in detail.rs to reuse or mirror, by their real names.
- Tests: the new tests in the existing styles, and which existing tests change
  (for example copy_returns_active_tab_full_text) and how.
- Anything the worker would otherwise have to guess, stated once.

Your source list is the evidence the worker will see. Every OLD block the worker
quotes must come from it. List ranges as one entry per line in the form
`grok:<repo-relative path>#L<start>-<end>`. Each range must start at an item's
first line (include its doc comment) and end at the line before the next item
in the map, so whole functions, structs and tests are included. Include every
item the worker must replace, every anchor it inserts beside, the read-only
helpers it must call, and the tests it must change. Each range is at most 32 KB.
Keep the list as small as the task allows.

If something the worker needs is not in the map or the excerpts, do not guess:
put it under QUESTIONS.

Return exactly the file block below. The markdown has five sections in this order:
- `SLICE_ID:` one line, kebab-case.
- `SOURCE_RANGES:` the entries, one per line, nothing else.
- `SPEC:` the complete task text for the worker.
- `RANGE_NOTES:` one line per range saying which items it covers and why.
- `QUESTIONS:` open questions for the coordinator, or `none`.

Return exactly:
=== FILE: task-scripts/acpx-disclosure-review/packetgen-v1/inspector-focus.packet.md ===
```markdown
<complete packet file>
```
